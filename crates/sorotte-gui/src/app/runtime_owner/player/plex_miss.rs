use super::*;

const PLEX_MISS_BACKOFF: [Duration; 4] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

pub(super) fn plex_miss_backoff(attempt_count: u32) -> Duration {
    let index = attempt_count
        .saturating_sub(1)
        .min((PLEX_MISS_BACKOFF.len() - 1) as u32) as usize;
    PLEX_MISS_BACKOFF[index]
}

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn reconcile_plex_miss_key(&mut self, key: &PlexResolutionMissKey) {
        if self
            .plex_miss_state
            .as_ref()
            .is_some_and(|state| &state.key != key)
        {
            self.plex_miss_state = None;
        }
    }

    pub(super) fn plex_resolution_allowed_now(
        &mut self,
        key: &PlexResolutionMissKey,
        now: Instant,
    ) -> bool {
        self.reconcile_plex_miss_key(key);
        let Some(state) = self.plex_miss_state.as_mut() else {
            return true;
        };
        if state.disposition == GuiPlexStreamResolveFailureDisposition::PermanentForContext {
            return false;
        }
        if state.retry_in_flight
            || state
                .next_retry_at
                .is_some_and(|next_retry_at| now < next_retry_at)
        {
            return false;
        }
        state.retry_in_flight = true;
        true
    }

    pub(super) fn record_plex_resolution_miss(&mut self, key: PlexResolutionMissKey, now: Instant) {
        let attempt_count = self
            .plex_miss_state
            .as_ref()
            .filter(|state| state.key == key)
            .map(|state| state.attempt_count.saturating_add(1))
            .unwrap_or(1);
        self.plex_miss_state = Some(PlexMissState {
            key,
            last_attempt_at: now,
            next_retry_at: Some(now + plex_miss_backoff(attempt_count)),
            attempt_count,
            retry_in_flight: false,
            disposition: GuiPlexStreamResolveFailureDisposition::Retryable,
        });
    }

    pub(super) fn record_permanent_plex_resolution_failure(
        &mut self,
        key: PlexResolutionMissKey,
        now: Instant,
    ) {
        let attempt_count = self
            .plex_miss_state
            .as_ref()
            .filter(|state| state.key == key)
            .map(|state| state.attempt_count.saturating_add(1))
            .unwrap_or(1);
        self.plex_miss_state = Some(PlexMissState {
            key,
            last_attempt_at: now,
            next_retry_at: None,
            attempt_count,
            retry_in_flight: false,
            disposition: GuiPlexStreamResolveFailureDisposition::PermanentForContext,
        });
    }

    pub(super) fn record_plex_resolution_failure(
        &mut self,
        key: PlexResolutionMissKey,
        disposition: GuiPlexStreamResolveFailureDisposition,
        now: Instant,
    ) {
        match disposition {
            GuiPlexStreamResolveFailureDisposition::Retryable => {
                self.record_plex_resolution_miss(key, now);
            }
            GuiPlexStreamResolveFailureDisposition::PermanentForContext => {
                self.record_permanent_plex_resolution_failure(key, now);
            }
        }
    }

    pub(super) fn clear_plex_resolution_miss_for_key(&mut self, key: &PlexResolutionMissKey) {
        if self
            .plex_miss_state
            .as_ref()
            .is_some_and(|state| &state.key == key)
        {
            self.plex_miss_state = None;
        }
    }

    pub(super) fn matching_plex_miss_retry_due(
        &self,
        key: &PlexResolutionMissKey,
        now: Instant,
    ) -> bool {
        self.plex_miss_state.as_ref().is_some_and(|state| {
            &state.key == key
                && state.disposition == GuiPlexStreamResolveFailureDisposition::Retryable
                && !state.retry_in_flight
                && state
                    .next_retry_at
                    .is_some_and(|next_retry_at| now >= next_retry_at)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miss_key(
        row_id: GuiPlaylistEntryId,
        generation: u64,
        target: &str,
    ) -> PlexResolutionMissKey {
        PlexResolutionMissKey {
            row_id,
            playlist_generation: generation,
            policy: GuiPlaylistSourcePolicy::Automatic,
            stream_trigger_key: target.to_owned(),
        }
    }

    #[test]
    fn miss_backoff_is_bounded_at_thirty_seconds() {
        assert_eq!(plex_miss_backoff(1), Duration::from_secs(2));
        assert_eq!(plex_miss_backoff(2), Duration::from_secs(5));
        assert_eq!(plex_miss_backoff(3), Duration::from_secs(15));
        assert_eq!(plex_miss_backoff(4), Duration::from_secs(30));
        assert_eq!(plex_miss_backoff(40), Duration::from_secs(30));
    }

    #[test]
    fn active_key_misses_follow_independent_backoff_and_reset_on_success_or_scope_change() {
        let row_id = GuiPlaylistEntryId::next();
        let key = miss_key(row_id, 3, "target-a");
        let other_key = miss_key(row_id, 3, "target-b");
        let start = Instant::now();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

        owner.record_plex_resolution_miss(key.clone(), start);
        let first = owner.plex_miss_state.as_ref().unwrap();
        assert_eq!(first.attempt_count, 1);
        assert_eq!(first.next_retry_at, Some(start + Duration::from_secs(2)));
        assert!(!owner.plex_resolution_allowed_now(&key, start + Duration::from_secs(1)));
        assert!(owner.plex_resolution_allowed_now(&key, start + Duration::from_secs(2)));
        assert!(owner.plex_miss_state.as_ref().unwrap().retry_in_flight);

        let second_attempt = start + Duration::from_secs(2);
        owner.record_plex_resolution_miss(key.clone(), second_attempt);
        let second = owner.plex_miss_state.as_ref().unwrap();
        assert_eq!(second.attempt_count, 2);
        assert_eq!(
            second.next_retry_at,
            Some(second_attempt + Duration::from_secs(5))
        );

        owner.clear_plex_resolution_miss_for_key(&key);
        assert!(owner.plex_miss_state.is_none());
        assert!(owner.plex_resolution_allowed_now(&key, second_attempt));

        owner.record_plex_resolution_miss(key, second_attempt);
        owner.reconcile_plex_miss_key(&other_key);
        assert!(owner.plex_miss_state.is_none());
    }

    #[test]
    fn permanent_failure_has_no_deadline_and_rearms_only_for_a_new_context() {
        let row_id = GuiPlaylistEntryId::next();
        let key = miss_key(row_id, 3, "target-a");
        let other_key = miss_key(row_id, 4, "target-a");
        let start = Instant::now();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

        owner.record_permanent_plex_resolution_failure(key.clone(), start);

        let failure = owner.plex_miss_state.as_ref().unwrap();
        assert_eq!(
            failure.disposition,
            GuiPlexStreamResolveFailureDisposition::PermanentForContext
        );
        assert_eq!(failure.attempt_count, 1);
        assert!(failure.next_retry_at.is_none());
        assert!(!failure.retry_in_flight);
        assert!(
            !owner.plex_resolution_allowed_now(&key, start + Duration::from_secs(24 * 60 * 60))
        );
        assert!(
            !owner.matching_plex_miss_retry_due(&key, start + Duration::from_secs(24 * 60 * 60))
        );

        assert!(owner.plex_resolution_allowed_now(&other_key, start));
        assert!(owner.plex_miss_state.is_none());
    }

    #[test]
    fn non_automatic_policy_clears_active_automatic_miss_retry() {
        let mut state = SorotteGuiShellAppState::from_stored_settings(
            &sorotte_client_app::app_boundary::state::StoredClientSettingsMvp {
                shared_playlist_enabled: Some(true),
                ..Default::default()
            },
        );
        state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
        state.main_window.active_playlist_index = Some(0);
        let row_id = state.main_window.playlist[0].entry_id;
        let now = Instant::now();
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.active_shared_playlist_index = Some(0);

        for policy in [
            GuiPlaylistSourcePolicy::ForceLocal,
            GuiPlaylistSourcePolicy::ForceMediaMatching,
            GuiPlaylistSourcePolicy::ForcePlex,
        ] {
            owner.plex_miss_state = Some(PlexMissState {
                key: miss_key(row_id, owner.playlist_resolution.generation, "episode.mkv"),
                last_attempt_at: now,
                next_retry_at: Some(now),
                attempt_count: 1,
                retry_in_flight: false,
                disposition: GuiPlexStreamResolveFailureDisposition::Retryable,
            });
            state.main_window.playlist[0].source_state.policy = policy;

            assert!(!owner.active_plex_miss_retry_due(&state));
            assert!(
                owner.plex_miss_state.is_none(),
                "{policy:?} must discard Automatic's independent retry state"
            );
        }
    }
}
