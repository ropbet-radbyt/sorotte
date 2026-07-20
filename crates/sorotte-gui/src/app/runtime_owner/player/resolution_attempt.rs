use super::*;

const FAILED_PLAYLIST_CANDIDATE_RETRY_DELAY: Duration = Duration::from_secs(5);

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
        self.playlist_resolution_attempt
            .as_ref()
            .map(|attempt| attempt.failed_candidates.clone())
            .unwrap_or_default()
    }

    pub(in crate::app::runtime_owner) fn active_playlist_candidate_retry_due(&self) -> bool {
        self.playlist_resolution_attempt
            .as_ref()
            .is_some_and(|attempt| {
                attempt.playlist_generation == self.playlist_resolution.generation
                    && attempt.state == PlaylistResolutionAttemptState::Failed
                    && attempt
                        .failed_candidate_retry_at
                        .is_some_and(|deadline| Instant::now() >= deadline)
            })
    }

    pub(super) fn reset_failed_playlist_candidates_if_retry_due(&mut self, now: Instant) -> bool {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return false;
        };
        if attempt.state != PlaylistResolutionAttemptState::Failed
            || attempt
                .failed_candidate_retry_at
                .is_none_or(|deadline| now < deadline)
        {
            return false;
        }
        attempt.failed_candidates.clear();
        attempt.failed_candidate_retry_at = None;
        attempt.candidate_provider = None;
        attempt.candidate = None;
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Resolving;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
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
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        attempt.candidate_provider = Some(provider_id);
        attempt.candidate = None;
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Active;
        attempt.failed_candidate_retry_at = None;
        attempt.fallback_pending = false;
        attempt.handoff_pending = false;
    }

    pub(super) fn fail_playlist_resolution_candidate(
        &mut self,
        candidate: media_resolution::GuiMediaResolutionCandidate,
    ) {
        let Some(attempt) = self.playlist_resolution_attempt.as_mut() else {
            return;
        };
        if !attempt.failed_candidates.contains(&candidate) {
            attempt.failed_candidates.push(candidate.clone());
        }
        attempt.candidate_provider = Some(candidate.provider_id());
        attempt.candidate = Some(candidate);
        attempt.player_command_id = None;
        attempt.player_media_generation = None;
        attempt.state = PlaylistResolutionAttemptState::Failed;
        attempt.failed_candidate_retry_at =
            Some(Instant::now() + FAILED_PLAYLIST_CANDIDATE_RETRY_DELAY);
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
                    attempt.state = PlaylistResolutionAttemptState::Active;
                    attempt.failed_candidate_retry_at = None;
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
            PlayerCommandResult::Failed(_) => {
                let failed_candidate = self
                    .playlist_resolution_attempt
                    .as_ref()
                    .and_then(|attempt| attempt.candidate.clone());
                if let Some(candidate) = failed_candidate {
                    self.clear_rejected_resolution_placeholder(&candidate);
                    self.fail_playlist_resolution_candidate(candidate);
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

    pub(super) fn handle_untracked_playlist_media_load_outcome(
        &mut self,
        outcome: &PlayerMediaLoadOutcome,
    ) {
        let matching_candidate = self
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| {
                (attempt.player_command_id.is_none()
                    && attempt.state == PlaylistResolutionAttemptState::Loading)
                    .then_some(attempt.candidate.as_ref())
                    .flatten()
                    .filter(|candidate| candidate.matches_loaded_target(&outcome.requested_target))
                    .cloned()
            });
        let Some(candidate) = matching_candidate else {
            return;
        };
        if outcome.succeeded() {
            if let Some(attempt) = self.playlist_resolution_attempt.as_mut() {
                attempt.state = PlaylistResolutionAttemptState::Active;
                attempt.failed_candidate_retry_at = None;
                attempt.fallback_pending = false;
                attempt.handoff_pending = true;
            }
            self.last_attached_media_resolution_trigger = None;
        } else {
            self.fail_playlist_resolution_candidate(candidate);
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
            attempt.state = PlaylistResolutionAttemptState::Active;
            attempt.failed_candidate_retry_at = None;
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
                .failed_candidates
                .iter()
                .map(|candidate| {
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
            .failed_candidates
            .iter()
            .map(|candidate| {
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
    use crate::app::runtime_owner::GuiPendingLogicalMediaOverride;
    use crate::app::runtime_owner::player::media_resolution::GuiMediaResolutionPlan;
    use sorotte_player_api::{PlayerCommandFailureKind, PlayerCommandId};

    fn local_candidate(path: &str) -> media_resolution::GuiMediaResolutionCandidate {
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_user_media_candidate(
            path.to_owned(),
            GuiUserMediaTargetResolutionSource::QuickLocal,
        );
        plan.best_candidate().cloned().expect("local candidate")
    }

    fn started(command_id: u64) -> StartedMediaLoad {
        StartedMediaLoad {
            feedback_message: "started".to_owned(),
            player_command_id: Some(PlayerCommandId::new(command_id)),
            player_media_generation: None,
        }
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
            owner.open_media_resolution_candidate("episode.mkv", candidate, false),
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
        assert_eq!(attempt.failed_candidates, vec![candidate]);
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
