use super::*;

fn emit_media_resolution_transition(
    transition: &'static str,
    trigger: sorotte_lifecycle_evidence::Trigger,
    disposition: sorotte_lifecycle_evidence::Disposition,
    playlist_generation: u64,
) {
    crate::emit_gui_lifecycle_transition(
        crate::GuiLifecycleOrigin::new(
            sorotte_lifecycle_evidence::ProcessRole::Client,
            "gui-media-resolver",
        ),
        transition,
        "media-resolution",
        sorotte_lifecycle_evidence::TargetKind::GuiProjection,
        trigger,
        disposition,
        &[("playlist-generation", playlist_generation.max(1))],
    );
}

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
                                | PlaylistResolutionAttemptState::Indeterminate
                        )
                });
        !self.player_local_file_placeholder && !playlist_load_unconfirmed
    }

    /// Returns whether the selected successor has generation-correlated
    /// `file-loaded` evidence even though its tracked load command has not yet
    /// emitted its terminal bookkeeping event.
    ///
    /// A successful media outcome is the physical fact needed to apply the
    /// playlist pause-and-rewind handoff. Keeping the attempt in `Loading`
    /// until command completion still protects normal attached-player sync and
    /// preserves command ownership, but must not strand the selection fence or
    /// let its corrective Pause race a later user Play.
    pub(in crate::app::runtime_owner) fn player_media_confirmed_for_pending_playlist_reset(
        &self,
        target: &str,
    ) -> bool {
        self.player_local_file_ready_for_attached_sync()
            || (self.player_local_file.is_some()
                && self.current_player_is_loading_media_target(target)
                && self
                    .playlist_resolution_attempt
                    .as_ref()
                    .is_some_and(|attempt| {
                        attempt.playlist_generation == self.playlist_resolution.generation
                            && attempt.target == target
                            && attempt.state == PlaylistResolutionAttemptState::Loading
                            && !attempt.media_confirmation_pending
                    }))
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

        let media_confirmation_pending =
            self.playlist_resolution_attempt
                .as_ref()
                .is_some_and(|attempt| {
                    attempt.playlist_generation == playlist_generation
                        && attempt.target == target
                        && attempt.policy == policy
                        && matches!(
                            attempt.state,
                            PlaylistResolutionAttemptState::Loading
                                | PlaylistResolutionAttemptState::Indeterminate
                        )
                        && attempt.media_confirmation_pending
                });
        let retired_generation = self
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.playlist_generation);
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.state = PlaylistResolutionAttemptState::Superseded;
        }
        if let Some(generation) = retired_generation {
            emit_media_resolution_transition(
                "MEDIA-CLEAR-001",
                sorotte_lifecycle_evidence::Trigger::RemoteEvent,
                sorotte_lifecycle_evidence::Disposition::Superseded,
                generation,
            );
        }
        self.pending_logical_media_override = None;
        let mut replacement =
            PlaylistResolutionAttempt::new(row_id, playlist_generation, target.to_owned(), policy);
        replacement.media_confirmation_pending = media_confirmation_pending;
        self.playlist_resolution_attempt = Some(replacement);
        emit_media_resolution_transition(
            "MEDIA-SELECT-001",
            sorotte_lifecycle_evidence::Trigger::RemoteEvent,
            sorotte_lifecycle_evidence::Disposition::Applied,
            playlist_generation,
        );
        emit_media_resolution_transition(
            "MEDIA-RESOLVE-001",
            sorotte_lifecycle_evidence::Trigger::Internal,
            sorotte_lifecycle_evidence::Disposition::Submitted,
            playlist_generation,
        );
    }

    pub(in crate::app::runtime_owner) fn supersede_playlist_resolution_attempt(&mut self) {
        let retired_generation = self
            .playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.playlist_generation);
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.state = PlaylistResolutionAttemptState::Superseded;
        }
        self.playlist_resolution_attempt = None;
        self.pending_logical_media_override = None;
        if let Some(generation) = retired_generation {
            emit_media_resolution_transition(
                "MEDIA-CLEAR-001",
                sorotte_lifecycle_evidence::Trigger::Internal,
                sorotte_lifecycle_evidence::Disposition::Applied,
                generation,
            );
        }
    }

    pub(super) fn record_playlist_resolution_missing(&mut self) {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        if attempt.state != PlaylistResolutionAttemptState::Resolving || attempt.missing_reported {
            return;
        }
        attempt.missing_reported = true;
        emit_media_resolution_transition(
            "MEDIA-MISSING-001",
            sorotte_lifecycle_evidence::Trigger::Internal,
            sorotte_lifecycle_evidence::Disposition::Applied,
            attempt.playlist_generation,
        );
    }

    pub(super) fn record_playlist_resolution_untrusted(&mut self) {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        if attempt.state != PlaylistResolutionAttemptState::Resolving || attempt.untrusted_reported
        {
            return;
        }
        attempt.untrusted_reported = true;
        emit_media_resolution_transition(
            "MEDIA-UNTRUSTED-001",
            sorotte_lifecycle_evidence::Trigger::Internal,
            sorotte_lifecycle_evidence::Disposition::Rejected,
            attempt.playlist_generation,
        );
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
                let mut excluded = attempt
                    .candidate_failures
                    .iter()
                    .filter(|failure| {
                        !due_transient_is_eligible || failure.excludes_candidate_at(now)
                    })
                    .map(|failure| failure.candidate.clone())
                    .collect::<Vec<_>>();
                if attempt.state == PlaylistResolutionAttemptState::Indeterminate
                    && let Some(candidate) = attempt.candidate.as_ref()
                    && !excluded.contains(candidate)
                {
                    // A timeout is not a permanent candidate failure, but one
                    // fallback pass must not immediately submit the same
                    // still-live physical attempt again.
                    excluded.push(candidate.clone());
                }
                excluded
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
        attempt.load_attempt_id = None;
        attempt.media_confirmation_pending = false;
        attempt.state = PlaylistResolutionAttemptState::Resolving;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
        attempt.missing_reported = false;
        attempt.untrusted_reported = false;
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
        let playlist_generation = attempt.playlist_generation;
        self.last_attached_media_resolution_trigger = None;
        emit_media_resolution_transition(
            "MEDIA-RESOLVE-001",
            sorotte_lifecycle_evidence::Trigger::Recovery,
            sorotte_lifecycle_evidence::Disposition::Submitted,
            playlist_generation,
        );
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
        let retried = attempt.state == PlaylistResolutionAttemptState::Failed;
        if retried {
            Self::prepare_failed_attempt_for_retry(attempt);
        }
        let playlist_generation = attempt.playlist_generation;
        self.last_attached_media_resolution_trigger = None;
        if retried {
            emit_media_resolution_transition(
                "MEDIA-RESOLVE-001",
                sorotte_lifecycle_evidence::Trigger::LocalInput,
                sorotte_lifecycle_evidence::Disposition::Submitted,
                playlist_generation,
            );
        }
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
        attempt.load_attempt_id = None;
        attempt.media_confirmation_pending = started.player_command_id.is_some();
        attempt.state = PlaylistResolutionAttemptState::Loading;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
        attempt.missing_reported = false;
        attempt.untrusted_reported = false;
    }

    #[cfg(test)]
    pub(in crate::app::runtime_owner) fn open_plex_resolution_candidate_for_test(
        &mut self,
        row_id: GuiPlaylistEntryId,
        playlist_generation: u64,
        target: &str,
        policy: GuiPlaylistSourcePolicy,
        stream_target: sorotte_plex::PlexStreamTarget,
    ) -> Option<Result<StartedMediaLoad, String>> {
        self.ensure_playlist_resolution_attempt(row_id, playlist_generation, target, policy);
        let mut plan = media_resolution::GuiMediaResolutionPlan::new(target);
        plan.push_plex_stream_candidate(stream_target.clone());
        let candidate = plan
            .best_candidate()
            .cloned()
            .expect("test Plex resolution plan should contain its stream candidate");
        let result = self.open_plex_stream_target_through_attached_player_result_impl(
            target,
            stream_target,
            false,
        );
        if let Some(Ok(started)) = result.as_ref() {
            self.begin_playlist_resolution_candidate_load(candidate, started);
        }
        result
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
        attempt.load_attempt_id = None;
        attempt.media_confirmation_pending = false;
        attempt.state = PlaylistResolutionAttemptState::Active;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
        let playlist_generation = attempt.playlist_generation;
        emit_media_resolution_transition(
            "MEDIA-PLAYABLE-001",
            sorotte_lifecycle_evidence::Trigger::PlayerEvent,
            sorotte_lifecycle_evidence::Disposition::Applied,
            playlist_generation,
        );
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
        attempt.load_attempt_id = None;
        attempt.state = PlaylistResolutionAttemptState::Failed;
        attempt.fallback_pending = true;
        attempt.handoff_pending = false;
        let playlist_generation = attempt.playlist_generation;
        self.last_attached_media_resolution_trigger = None;
        emit_media_resolution_transition(
            "MEDIA-FAIL-001",
            sorotte_lifecycle_evidence::Trigger::PlayerEvent,
            sorotte_lifecycle_evidence::Disposition::Failed,
            playlist_generation,
        );
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
                    && matches!(
                        attempt.state,
                        PlaylistResolutionAttemptState::Loading
                            | PlaylistResolutionAttemptState::Indeterminate
                    )
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
                    && matches!(
                        attempt.state,
                        PlaylistResolutionAttemptState::Loading
                            | PlaylistResolutionAttemptState::Indeterminate
                    )
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

    pub(super) fn track_playlist_resolution_load_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) -> bool {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        if attempt.playlist_generation != self.playlist_resolution.generation
            || !Self::player_progress_generation_matches(
                attempt.player_media_generation,
                Some(media_generation),
            )
            || attempt
                .player_command_id
                .zip(command_id)
                .is_some_and(|(expected, observed)| expected != observed)
            || attempt
                .load_attempt_id
                .is_some_and(|expected| expected != attempt_id)
            || matches!(
                attempt.state,
                PlaylistResolutionAttemptState::Failed | PlaylistResolutionAttemptState::Superseded
            )
        {
            return false;
        }
        if attempt.player_media_generation.is_none() {
            attempt.player_media_generation = Some(media_generation);
        }
        attempt.load_attempt_id = Some(attempt_id);
        if command_id.is_some() {
            attempt.media_confirmation_pending = true;
        }
        true
    }

    pub(super) fn mark_playlist_resolution_load_indeterminate(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) {
        if !self.track_playlist_resolution_load_attempt(attempt_id, media_generation, command_id) {
            return;
        }
        if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
            attempt.state = PlaylistResolutionAttemptState::Indeterminate;
            attempt.fallback_pending = true;
            attempt.handoff_pending = false;
        }
        self.last_attached_media_resolution_trigger = None;
    }

    pub(super) fn recover_playlist_resolution_from_active_load(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
        command_id: Option<PlayerCommandId>,
    ) -> bool {
        if !self.track_playlist_resolution_load_attempt(attempt_id, media_generation, command_id) {
            return false;
        }
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        if let Some(candidate) = attempt.candidate.as_ref() {
            attempt
                .candidate_failures
                .retain(|failure| &failure.candidate != candidate);
        }
        attempt.state = PlaylistResolutionAttemptState::Active;
        attempt.candidate_plex_operation_context = None;
        attempt.media_confirmation_pending = false;
        attempt.fallback_pending = false;
        attempt.handoff_pending = true;
        let playlist_generation = attempt.playlist_generation;
        emit_media_resolution_transition(
            "MEDIA-PLAYABLE-001",
            sorotte_lifecycle_evidence::Trigger::Recovery,
            sorotte_lifecycle_evidence::Disposition::Applied,
            playlist_generation,
        );

        let mut logical_override_confirmed = None;
        if let Some(pending) = self.pending_logical_media_override.as_mut()
            && pending.player_command_id == command_id
            && Self::player_progress_generation_matches(
                pending.player_media_generation,
                Some(media_generation),
            )
        {
            pending.load_completed = true;
            logical_override_confirmed = pending
                .logical_file_observed
                .then(|| pending.logical_file.clone());
        }
        if logical_override_confirmed
            .as_ref()
            .is_some_and(|logical_file| self.player_local_file.as_ref() == Some(logical_file))
        {
            self.player_local_file_placeholder = false;
        }
        self.last_attached_media_resolution_trigger = None;
        true
    }

    pub(super) fn supersede_playlist_resolution_load_attempt(
        &mut self,
        attempt_id: LoadAttemptId,
        media_generation: PlayerMediaGeneration,
    ) {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        if attempt.load_attempt_id != Some(attempt_id)
            || attempt.player_media_generation != Some(media_generation)
        {
            return;
        }
        attempt.state = PlaylistResolutionAttemptState::Superseded;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
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
                    (false, confirmed, None)
                }
                PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
                    PlayerCommandFailureKind::TimedOut,
                )) => {
                    // Command observation timed out, but the correlated
                    // physical load remains capable of becoming active.
                    (false, None, None)
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

        let (terminal_result, attempt_was_active, media_was_confirmed) = {
            let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
                return;
            };
            if attempt.player_command_id != Some(progress.command_id)
                || attempt.playlist_generation != self.playlist_resolution.generation
                || attempt.state == PlaylistResolutionAttemptState::Superseded
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
                PlayerCommandProgressState::Finished(result) => (
                    result,
                    attempt.state == PlaylistResolutionAttemptState::Active,
                    !attempt.media_confirmation_pending,
                ),
            }
        };

        match terminal_result {
            PlayerCommandResult::Completed => {
                let confirmed_current_player = !self.player_local_file_placeholder
                    && self
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
                let load_is_active =
                    attempt_was_active || media_was_confirmed || confirmed_current_player;
                if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                    // The IPC reply only confirms that mpv accepted loadfile.
                    // Retire command correlation, but keep the physical load
                    // pending until a matching media identity or active-load
                    // lifecycle event confirms it. This also lets a later
                    // terminal media outcome classify the accepted load.
                    attempt.player_command_id = None;
                    if load_is_active {
                        if let Some(candidate) = attempt.candidate.as_ref() {
                            attempt
                                .candidate_failures
                                .retain(|failure| &failure.candidate != candidate);
                        }
                        attempt.state = PlaylistResolutionAttemptState::Active;
                        attempt.candidate_plex_operation_context = None;
                        attempt.media_confirmation_pending = false;
                        attempt.fallback_pending = false;
                        attempt.handoff_pending = true;
                    } else {
                        attempt.state = PlaylistResolutionAttemptState::Loading;
                        attempt.media_confirmation_pending = true;
                        attempt.fallback_pending = false;
                        attempt.handoff_pending = false;
                    }
                }
                if load_is_active {
                    self.player_local_file_placeholder = false;
                }
                if load_is_active {
                    self.last_attached_media_resolution_trigger = None;
                }
            }
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut) => {
                if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                    if attempt_was_active || media_was_confirmed {
                        // A missing command terminal is uncertainty, not
                        // negative evidence. Once the correlated physical
                        // load is known active, preserve that stronger fact
                        // instead of fencing a playing client back out of
                        // room synchronization.
                        attempt.state = PlaylistResolutionAttemptState::Active;
                        attempt.media_confirmation_pending = false;
                        attempt.fallback_pending = false;
                        attempt.handoff_pending = true;
                    } else {
                        attempt.state = PlaylistResolutionAttemptState::Indeterminate;
                        attempt.fallback_pending = true;
                        attempt.handoff_pending = false;
                    }
                }
                if attempt_was_active || media_was_confirmed {
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
        self.handle_playlist_media_load_outcome_for_generation(outcome, None);
    }

    pub(super) fn handle_playlist_media_load_outcome_for_generation(
        &mut self,
        outcome: &PlayerMediaLoadOutcome,
        observed_media_generation: Option<PlayerMediaGeneration>,
    ) {
        let matching_load = self
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| {
                if !matches!(
                    attempt.state,
                    PlaylistResolutionAttemptState::Loading
                        | PlaylistResolutionAttemptState::Indeterminate
                ) || observed_media_generation.is_some_and(|observed| {
                    attempt
                        .player_media_generation
                        .is_some_and(|expected| expected != observed)
                }) {
                    return None;
                }
                attempt
                    .candidate
                    .as_ref()
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
        if let Some(observed) = observed_media_generation
            && let Some(attempt) = self.playlist_resolution_attempt.as_mut()
            && attempt.player_media_generation.is_none()
        {
            attempt.player_media_generation = Some(observed);
        }
        if outcome.succeeded() {
            // A tracked command remains Loading until its correlated command
            // completion arrives. Retain the positive physical evidence so
            // the later terminal cannot lose it merely because the adapter
            // delivered file-loaded first.
            if tracked {
                let player_command_id = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.player_command_id);
                if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                    attempt.media_confirmation_pending = false;
                }
                if let Some(pending) = self.pending_logical_media_override.as_mut()
                    && pending.player_command_id == player_command_id
                    && observed_media_generation.is_none_or(|observed| {
                        pending
                            .player_media_generation
                            .is_none_or(|expected| expected == observed)
                    })
                {
                    if pending.player_media_generation.is_none() {
                        pending.player_media_generation = observed_media_generation;
                    }
                    pending.load_completed = true;
                }
                return;
            }
            if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                attempt
                    .candidate_failures
                    .retain(|failure| failure.candidate != candidate);
                attempt.state = PlaylistResolutionAttemptState::Active;
                attempt.candidate_plex_operation_context = None;
                attempt.media_confirmation_pending = false;
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
        self.handle_playlist_local_file_observation(update, false);
    }

    pub(super) fn handle_authoritative_playlist_local_file_observation(
        &mut self,
        update: &LocalFileUpdate,
    ) {
        self.handle_playlist_local_file_observation(update, true);
    }

    fn handle_playlist_local_file_observation(
        &mut self,
        update: &LocalFileUpdate,
        physical_file_loaded: bool,
    ) {
        let matching_observation =
            self.playlist_resolution_attempt
                .as_ref()
                .is_some_and(|attempt| {
                    attempt.player_command_id.is_none()
                        && (physical_file_loaded || !attempt.media_confirmation_pending)
                        && matches!(
                            attempt.state,
                            PlaylistResolutionAttemptState::Loading
                                | PlaylistResolutionAttemptState::Indeterminate
                        )
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
            attempt.media_confirmation_pending = false;
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
            let permanent_plex_failure = matching_plex_miss.is_some_and(|miss| {
                miss.disposition == GuiPlexStreamResolveFailureDisposition::PermanentForContext
            });
            let resolution_in_flight = self.attached_media_search_in_flight()
                || self.media_match_remote_lookup_rx.is_some()
                || self.plex_stream_resolution_owns_cache_snapshot()
                || matching_plex_miss.is_some_and(|miss| miss.retry_in_flight);
            source_state.status = if resolution_in_flight {
                GuiPlaylistSourceStatus::Resolving
            } else if permanent_plex_failure {
                GuiPlaylistSourceStatus::Failed
            } else {
                GuiPlaylistSourceStatus::Missing
            };
            source_state.detail = Some(if resolution_in_flight {
                "Searching the available media providers.".to_owned()
            } else if permanent_plex_failure {
                "Plex found multiple indistinguishable playable parts; choose a source or retry after changing Plex metadata.".to_owned()
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
            PlaylistResolutionAttemptState::Indeterminate => GuiPlaylistSourceStatus::Loading,
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
            PlaylistResolutionAttemptState::Indeterminate => {
                format!(
                    "The attached player has not confirmed the {provider_label} load yet; checking fallback sources."
                )
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
        owner.player_local_file = Some(
            LocalFileUpdate::new("healthy-fallback.mkv").with_path("C:/media/healthy-fallback.mkv"),
        );
        owner.player_local_file_placeholder = false;
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
    fn completed_command_before_active_media_stays_loading_until_media_confirmation() {
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
            PlaylistResolutionAttemptState::Loading
        );
        assert_eq!(
            owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .player_command_id,
            None,
            "the terminal IPC response should retire command correlation without confirming media"
        );
        assert!(
            !owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .handoff_pending
        );
        assert!(
            owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .media_confirmation_pending
        );

        owner.handle_untracked_playlist_local_file_observation(
            &LocalFileUpdate::new("episode.mkv").with_path("C:/media/episode.mkv"),
        );
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(
            attempt.state,
            PlaylistResolutionAttemptState::Loading,
            "a matching path observation can arrive while mpv is only opening and must not substitute for file-loaded"
        );
        assert!(attempt.media_confirmation_pending);
        assert!(!attempt.handoff_pending);

        owner.handle_playlist_media_load_outcome(&PlayerMediaLoadOutcome::success(
            "C:/media/episode.mkv",
            Some("C:/media/episode.mkv".to_owned()),
        ));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert!(!attempt.media_confirmation_pending);
        assert!(attempt.handoff_pending);
    }

    #[test]
    fn matching_media_success_before_completed_command_is_not_discarded() {
        let row_id = GuiPlaylistEntryId::next();
        let path = "C:/media/episode.mkv";
        let candidate = local_candidate(path);
        let command_id = PlayerCommandId::new(12);
        let media_generation = PlayerMediaGeneration::new(4);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 10;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            10,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(command_id.get()));
        owner.player_local_file = Some(LocalFileUpdate::new("episode.mkv").with_path(path));
        owner.player_local_file_placeholder = true;
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(media_generation),
            None,
        ));
        assert!(
            !owner.player_media_confirmed_for_pending_playlist_reset("episode.mkv"),
            "an accepted load command is not physical file-loaded evidence"
        );

        let success = PlayerMediaLoadOutcome::success(path, Some(path.to_owned()));
        for mismatched_generation in [
            PlayerMediaGeneration::new(media_generation.get() - 1),
            PlayerMediaGeneration::new(media_generation.get() + 1),
        ] {
            owner.handle_playlist_media_load_outcome_for_generation(
                &success,
                Some(mismatched_generation),
            );
            assert!(
                owner
                    .playlist_resolution_attempt
                    .as_ref()
                    .expect("tracked attempt")
                    .media_confirmation_pending,
                "a different media generation cannot confirm the tracked load"
            );
            assert!(
                !owner.player_media_confirmed_for_pending_playlist_reset("episode.mkv"),
                "a mismatched generation cannot open the physical handoff fence"
            );
        }

        owner.handle_playlist_media_load_outcome_for_generation(&success, Some(media_generation));

        let provisional = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(provisional.state, PlaylistResolutionAttemptState::Loading);
        assert!(
            !provisional.media_confirmation_pending,
            "matching file-loaded evidence must be retained while the command terminal is pending"
        );
        assert!(!provisional.handoff_pending);
        assert!(owner.player_local_file_placeholder);
        assert!(
            owner.player_media_confirmed_for_pending_playlist_reset("episode.mkv"),
            "generation-correlated file-loaded evidence must open the physical reset fence before unrelated terminal bookkeeping"
        );
        assert!(
            !owner.player_media_confirmed_for_pending_playlist_reset("other-episode.mkv"),
            "file-loaded evidence remains scoped to the selected target"
        );

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(media_generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));

        let active = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(active.state, PlaylistResolutionAttemptState::Active);
        assert!(!active.media_confirmation_pending);
        assert!(active.handoff_pending);
        assert!(!owner.player_local_file_placeholder);
        assert!(owner.player_local_file_ready_for_attached_sync());
    }

    #[test]
    fn file_loaded_before_command_terminal_completes_the_playlist_reset_handoff() {
        let path = "C:/media/episode.mkv";
        let command_id = PlayerCommandId::new(13);
        let media_generation = PlayerMediaGeneration::new(5);
        let stored_settings = StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        };
        let mut state = SorotteGuiShellAppState::from_stored_settings(&stored_settings);
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        state.main_window.active_playlist_index = Some(0);
        let row_id = state.main_window.playlist[0].entry_id;

        let (mut owner, _session_transport) =
            GuiPersistedConfigRuntimeOwner::with_config_path(None)
                .with_client_core_chat_session_runtime("alice", "room1")
                .expect("client-core chat runtime should bootstrap");
        owner.active_session_settings = Some(
            sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
                &stored_settings,
            ),
        );
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
            GuiTestPlayerAdapter::default(),
        )));
        owner
            .session
            .as_mut()
            .expect("session should exist")
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            )
            .expect("hello should apply");
        owner
            .session
            .as_mut()
            .expect("session should exist")
            .note_local_playlist_index_reset_intent(true);

        owner.reconcile_local_shared_playlist_media_paths(&state);
        let playlist_generation = owner.playlist_resolution.generation;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            playlist_generation,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(local_candidate(path), &started(13));
        owner.player_local_file = Some(LocalFileUpdate::new("episode.mkv").with_path(path));
        owner.player_local_file_placeholder = true;
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(media_generation),
            None,
        ));
        owner.handle_playlist_media_load_outcome_for_generation(
            &PlayerMediaLoadOutcome::success(path, Some(path.to_owned())),
            Some(media_generation),
        );
        owner
            .session
            .as_mut()
            .expect("session should exist")
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-selection State should apply");

        let selected_media_sync =
            owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
        assert_eq!(
            selected_media_sync,
            SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget,
            "generation-correlated file-loaded evidence should make the selected successor eligible for its reset"
        );
        let handoff_ready = selected_media_sync.selection_handoff_ready(
            owner
                .session
                .as_ref()
                .expect("session should exist")
                .has_pending_playlist_index_reset_intent(),
        );
        assert!(handoff_ready);

        owner.apply_pending_playlist_index_reset_to_attached_player_impl(&state, handoff_ready);

        assert!(
            !owner
                .session
                .as_ref()
                .expect("session should exist")
                .has_pending_playlist_index_reset_intent(),
            "the completed physical handoff must release the coordinator transport hold"
        );
        assert_eq!(owner.player_position_seconds, Some(0.0));
        assert_eq!(owner.player_paused, Some(true));
        let attempt = owner
            .playlist_resolution_attempt
            .as_ref()
            .expect("tracked attempt should remain");
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
        assert!(
            owner.player_local_file_placeholder,
            "ordinary attached sync remains fenced until the independent command terminal arrives"
        );
    }

    #[test]
    fn loaded_target_confirmation_binds_missing_media_generation() {
        let row_id = GuiPlaylistEntryId::next();
        let command_id = PlayerCommandId::new(14);
        let media_generation = PlayerMediaGeneration::new(6);
        let stream_target = "https://plex.example/video?token=secret";
        let logical_file =
            LocalFileUpdate::new("episode.mkv").with_path("plex://machine/metadata/123");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 10;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            10,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner
            .begin_playlist_resolution_candidate_load(plex_candidate(), &started(command_id.get()));
        owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(stream_target),
            logical_file,
            user_initiated: false,
            player_command_id: Some(command_id),
            player_media_generation: None,
            playlist_row_id: Some(row_id),
            playlist_generation: 10,
            load_completed: false,
            logical_file_observed: false,
        });

        owner.handle_playlist_media_load_outcome_for_generation(
            &PlayerMediaLoadOutcome::success(
                "opaque-adapter-request",
                Some(stream_target.to_owned()),
            ),
            Some(media_generation),
        );

        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
        assert_eq!(attempt.player_media_generation, Some(media_generation));
        assert!(!attempt.media_confirmation_pending);
        let pending = owner.pending_logical_media_override.as_ref().unwrap();
        assert_eq!(pending.player_media_generation, Some(media_generation));
        assert!(pending.load_completed);
    }

    #[test]
    fn positive_physical_load_evidence_survives_a_late_command_timeout() {
        for active_lifecycle_event_observed in [false, true] {
            let row_id = GuiPlaylistEntryId::next();
            let path = "C:/media/episode.mkv";
            let candidate = local_candidate(path);
            let command_id = PlayerCommandId::new(13);
            let media_generation = PlayerMediaGeneration::new(5);
            let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
            owner.playlist_resolution.generation = 11;
            owner.ensure_playlist_resolution_attempt(
                row_id,
                11,
                "episode.mkv",
                GuiPlaylistSourcePolicy::Automatic,
            );
            owner.begin_playlist_resolution_candidate_load(candidate, &started(command_id.get()));
            owner.player_local_file = Some(LocalFileUpdate::new("episode.mkv").with_path(path));
            owner.player_local_file_placeholder = true;
            owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
                command_id,
                Some(media_generation),
                None,
            ));

            if active_lifecycle_event_observed {
                assert!(owner.recover_playlist_resolution_from_active_load(
                    LoadAttemptId::new(8),
                    media_generation,
                    Some(command_id),
                ));
            } else {
                owner.handle_playlist_media_load_outcome(&PlayerMediaLoadOutcome::success(
                    path,
                    Some(path.to_owned()),
                ));
            }

            owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
                command_id,
                Some(media_generation),
                None,
                None,
                PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
            ));

            let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
            assert_eq!(
                attempt.state,
                PlaylistResolutionAttemptState::Active,
                "a timeout must not erase stronger physical evidence when active_lifecycle_event_observed={active_lifecycle_event_observed}"
            );
            assert!(!attempt.media_confirmation_pending);
            assert!(!attempt.fallback_pending);
            assert!(attempt.handoff_pending);
            assert!(!owner.player_local_file_placeholder);
            assert!(owner.player_local_file_ready_for_attached_sync());
        }
    }

    #[test]
    fn active_media_before_completed_command_remains_active() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = local_candidate("C:/media/episode.mkv");
        let command_id = PlayerCommandId::new(11);
        let media_generation = PlayerMediaGeneration::new(3);
        let load_attempt_id = LoadAttemptId::new(7);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 9;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            9,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(command_id.get()));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(media_generation),
            None,
        ));
        assert!(owner.recover_playlist_resolution_from_active_load(
            load_attempt_id,
            media_generation,
            Some(command_id),
        ));
        assert_eq!(
            owner.playlist_resolution_attempt.as_ref().unwrap().state,
            PlaylistResolutionAttemptState::Active
        );

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(media_generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert_eq!(attempt.player_command_id, None);
        assert!(attempt.handoff_pending);
    }

    #[test]
    fn late_completed_command_cannot_revive_superseded_load_attempt() {
        let row_id = GuiPlaylistEntryId::next();
        let command_id = PlayerCommandId::new(12);
        let media_generation = PlayerMediaGeneration::new(4);
        let load_attempt_id = LoadAttemptId::new(8);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 9;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            9,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(
            local_candidate("C:/media/episode.mkv"),
            &started(command_id.get()),
        );
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(media_generation),
            None,
        ));
        assert!(owner.track_playlist_resolution_load_attempt(
            load_attempt_id,
            media_generation,
            Some(command_id),
        ));
        owner.supersede_playlist_resolution_load_attempt(load_attempt_id, media_generation);

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(media_generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Superseded);
        assert!(!attempt.handoff_pending);
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
    fn authoritative_completed_load_replay_keeps_matching_candidate_active() {
        let row_id = GuiPlaylistEntryId::next();
        let target = "C:/media/episode.mkv";
        let candidate = local_candidate(target);
        let command_id = PlayerCommandId::new(11);
        let generation = PlayerMediaGeneration::new(3);
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 9;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            9,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(command_id.get()));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(generation),
            None,
        ));
        owner.player_local_file = Some(LocalFileUpdate::new("episode.mkv").with_path(target));
        owner.player_local_file_placeholder = true;
        owner.attached_media_observation_cursor.media_generation = Some(generation.get());

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(generation),
            None,
            None,
            PlayerCommandResult::Completed,
        ));
        let boundary = owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                LocalFileUpdate::new("episode.mkv").with_path(target),
                Some(generation),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(10)),
            true,
        );

        assert_eq!(boundary, None);
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert!(!attempt.fallback_pending);
        assert!(attempt.candidate_failures.is_empty());
        assert_eq!(
            owner
                .player_local_file
                .as_ref()
                .and_then(|file| file.path.as_deref()),
            Some(target)
        );
        assert!(!owner.player_local_file_placeholder);
    }

    #[test]
    fn authoritative_same_file_replay_activates_untracked_playlist_attempt() {
        let row_id = GuiPlaylistEntryId::next();
        let target = "C:/media/episode.mkv";
        let candidate = local_candidate(target);
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
        let update = LocalFileUpdate::new("episode.mkv").with_path(target);
        owner.player_local_file = Some(update.clone());
        owner.player_local_file_placeholder = true;
        owner.attached_media_observation_cursor.media_generation = Some(3);

        let boundary = owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                update,
                Some(PlayerMediaGeneration::new(3)),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(10)),
            true,
        );

        assert_eq!(boundary, None);
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert!(!attempt.fallback_pending);
        assert!(attempt.candidate_failures.is_empty());
        assert!(!owner.player_local_file_placeholder);
    }

    #[test]
    fn authoritative_same_generation_plex_updates_keep_logical_identity_until_newer_media() {
        let generation = PlayerMediaGeneration::new(7);
        let command_id = PlayerCommandId::new(22);
        let stream_target = "https://plex.example/stream?token=secret";
        let redirected_target = "https://redirected.plex.direct/stream?token=secret";
        let logical_file =
            LocalFileUpdate::new("episode.mkv").with_path("plex://machine/metadata/123");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(stream_target),
            logical_file: logical_file.clone(),
            user_initiated: false,
            player_command_id: Some(command_id),
            player_media_generation: Some(generation),
            playlist_row_id: None,
            playlist_generation: 0,
            load_completed: true,
            logical_file_observed: false,
        });
        owner.player_local_file = Some(logical_file.clone());
        owner.player_local_file_placeholder = true;
        owner.attached_media_observation_cursor.media_generation = Some(generation.get());

        let boundary = owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                LocalFileUpdate::new(stream_target).with_path(stream_target),
                Some(generation),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(10)),
            true,
        );

        assert_eq!(boundary, None);
        assert_eq!(owner.player_local_file, Some(logical_file.clone()));
        assert!(!owner.player_local_file_placeholder);
        assert!(owner.pending_logical_media_override.is_some());

        owner.player_position_seconds = Some(42.0);
        let redirected_boundary = owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                LocalFileUpdate::new(redirected_target)
                    .with_path(redirected_target)
                    .with_duration_seconds(90.0),
                Some(generation),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(11)),
            false,
        );

        assert_eq!(redirected_boundary, None);
        assert_eq!(owner.player_local_file, Some(logical_file));
        assert_eq!(owner.player_position_seconds, Some(42.0));
        assert!(owner.pending_logical_media_override.is_some());

        let external_target = "https://media.example/new-video.mkv";
        let newer_generation = PlayerMediaGeneration::new(generation.get() + 1);
        let external_boundary = owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                LocalFileUpdate::new(external_target).with_path(external_target),
                Some(newer_generation),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(12)),
            false,
        );

        assert!(external_boundary.is_some());
        assert_eq!(
            owner.player_local_file,
            Some(LocalFileUpdate::new(external_target).with_path(external_target))
        );
        assert_eq!(owner.player_position_seconds, Some(0.0));
        assert!(owner.pending_logical_media_override.is_none());
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
            PlaylistResolutionAttemptState::Loading,
            "the current command reply must still wait for matching media evidence"
        );
        assert_eq!(
            owner
                .playlist_resolution_attempt
                .as_ref()
                .unwrap()
                .player_command_id,
            None
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
    fn indeterminate_tracked_plex_load_recovers_on_late_active_and_matching_file() {
        let row_id = GuiPlaylistEntryId::next();
        let candidate = plex_candidate();
        let command_id = PlayerCommandId::new(5);
        let media_generation = PlayerMediaGeneration::new(8);
        let load_attempt_id = LoadAttemptId::new(11);
        let stream_target = "https://plex.example/video?token=secret";
        let logical_file = LocalFileUpdate::new("episode.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(candidate, &started(5));
        owner.player_local_file = Some(logical_file.clone());
        owner.player_local_file_placeholder = true;
        owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
            requested_target: "episode.mkv".to_owned(),
            loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(stream_target),
            logical_file: logical_file.clone(),
            user_initiated: false,
            player_command_id: Some(command_id),
            player_media_generation: None,
            playlist_row_id: Some(row_id),
            playlist_generation: 4,
            load_completed: false,
            logical_file_observed: false,
        });
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            command_id,
            Some(media_generation),
            None,
        ));
        assert!(owner.track_playlist_resolution_load_attempt(
            load_attempt_id,
            media_generation,
            Some(command_id),
        ));

        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::finished(
            command_id,
            Some(media_generation),
            None,
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
        ));
        owner.mark_playlist_resolution_load_indeterminate(
            load_attempt_id,
            media_generation,
            Some(command_id),
        );
        {
            let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
            assert_eq!(attempt.state, PlaylistResolutionAttemptState::Indeterminate);
            assert_eq!(attempt.load_attempt_id, Some(load_attempt_id));
            assert!(attempt.fallback_pending);
            assert!(attempt.candidate_failures.is_empty());
        }
        assert!(owner.pending_logical_media_override.is_some());
        assert!(owner.player_local_file_placeholder);

        assert!(owner.recover_playlist_resolution_from_active_load(
            load_attempt_id,
            media_generation,
            Some(command_id),
        ));
        owner.process_attached_local_file_observation(
            sorotte_player_api::PlayerLocalFileObservation::new(
                LocalFileUpdate::new(stream_target).with_path(stream_target),
                Some(media_generation),
                None,
            ),
            Some(sorotte_player_api::PlayerEventSequence::new(10)),
            true,
        );

        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Active);
        assert_eq!(attempt.load_attempt_id, Some(load_attempt_id));
        assert!(!attempt.fallback_pending);
        assert!(attempt.candidate_failures.is_empty());
        assert!(owner.failed_playlist_resolution_candidates().is_empty());
        assert_eq!(owner.player_local_file, Some(logical_file));
        assert!(!owner.player_local_file_placeholder);
        assert!(
            owner.pending_logical_media_override.is_some(),
            "the recovered active Plex generation must retain its logical projection"
        );
        assert!(owner.last_attached_media_resolution_trigger.is_none());
    }

    #[test]
    fn accepted_fallback_prevents_superseded_late_attempt_from_recovering_resolution() {
        let row_id = GuiPlaylistEntryId::next();
        let old_command = PlayerCommandId::new(5);
        let old_generation = PlayerMediaGeneration::new(8);
        let old_attempt = LoadAttemptId::new(11);
        let fallback_command = PlayerCommandId::new(6);
        let fallback_generation = PlayerMediaGeneration::new(9);
        let fallback_attempt = LoadAttemptId::new(12);
        let fallback_candidate = local_candidate("C:/media/fallback.mkv");
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.playlist_resolution.generation = 4;
        owner.ensure_playlist_resolution_attempt(
            row_id,
            4,
            "episode.mkv",
            GuiPlaylistSourcePolicy::Automatic,
        );
        owner.begin_playlist_resolution_candidate_load(
            local_candidate("C:/media/slow.mkv"),
            &started(5),
        );
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            old_command,
            Some(old_generation),
            None,
        ));
        assert!(owner.track_playlist_resolution_load_attempt(
            old_attempt,
            old_generation,
            Some(old_command),
        ));
        owner.mark_playlist_resolution_load_indeterminate(
            old_attempt,
            old_generation,
            Some(old_command),
        );
        owner.supersede_playlist_resolution_load_attempt(old_attempt, old_generation);

        owner.begin_playlist_resolution_candidate_load(fallback_candidate.clone(), &started(6));
        owner.handle_playlist_resolution_command_progress(PlayerCommandProgress::accepted(
            fallback_command,
            Some(fallback_generation),
            None,
        ));
        assert!(owner.track_playlist_resolution_load_attempt(
            fallback_attempt,
            fallback_generation,
            Some(fallback_command),
        ));

        assert!(!owner.recover_playlist_resolution_from_active_load(
            old_attempt,
            old_generation,
            Some(old_command),
        ));
        let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
        assert_eq!(attempt.load_attempt_id, Some(fallback_attempt));
        assert_eq!(attempt.player_command_id, Some(fallback_command));
        assert_eq!(attempt.candidate.as_ref(), Some(&fallback_candidate));
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
        assert!(
            owner.pending_logical_media_override.is_some(),
            "completion must retain the active generation's logical projection"
        );
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
