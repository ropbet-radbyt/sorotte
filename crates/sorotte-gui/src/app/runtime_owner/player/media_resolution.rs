use super::*;
use sorotte_plex::PlexStreamTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum GuiMediaResolutionPriority {
    CurrentPlayer,
    LocalExact,
    IndexedLocal,
    AlternateMatch,
    StreamFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum GuiMediaResolutionExecution {
    Synchronous,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum GuiMediaResolutionPhase {
    CurrentPlayer,
    ExactLocal,
    IndexedLocal,
    AlternateMatch,
    StreamFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum GuiMediaResolutionProviderKind {
    Core,
    MediaSearch,
    MediaMatch,
    Plex,
}

#[derive(Clone, PartialEq)]
pub(super) enum GuiMediaResolutionTarget {
    CurrentPlayer,
    LocalPath(String),
    PlexStream(Box<PlexStreamTarget>),
}

impl std::fmt::Debug for GuiMediaResolutionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentPlayer => formatter.write_str("CurrentPlayer"),
            Self::LocalPath(_) => formatter
                .debug_tuple("LocalPath")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::PlexStream(_) => formatter
                .debug_tuple("PlexStream")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::runtime_owner) struct GuiMediaResolutionCandidate {
    provider: GuiMediaResolutionProviderKind,
    phase: GuiMediaResolutionPhase,
    priority: GuiMediaResolutionPriority,
    execution: GuiMediaResolutionExecution,
    target: GuiMediaResolutionTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuiMediaResolutionPendingStep {
    provider: GuiMediaResolutionProviderKind,
    phase: GuiMediaResolutionPhase,
    priority: GuiMediaResolutionPriority,
    execution: GuiMediaResolutionExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiMediaResolutionFallbackPolicy {
    /// Preserve the declared resolution priority while higher-priority work is live.
    /// Provider workers own their timeout policy; once they finish or time out, their
    /// pending step disappears and the best ready fallback can be selected.
    WaitForHigherPriority,
    /// Select the best ready candidate without waiting for other providers. This is
    /// used for an explicit provider choice, where Automatic's fallback ordering does
    /// not apply.
    AllowReadyFallback,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum GuiMediaResolutionDecision {
    Ready(GuiMediaResolutionCandidate),
    WaitingForHigherPriority,
    Exhausted,
}

#[derive(Clone, PartialEq)]
pub(super) struct GuiMediaResolutionPlan {
    target: String,
    candidates: Vec<GuiMediaResolutionCandidate>,
    pending_steps: Vec<GuiMediaResolutionPendingStep>,
}

impl std::fmt::Debug for GuiMediaResolutionPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuiMediaResolutionPlan")
            .field("target", &sorotte_secret::REDACTED_SECRET)
            .field("candidates", &self.candidates)
            .field("pending_steps", &self.pending_steps)
            .finish()
    }
}

impl GuiMediaResolutionPlan {
    pub(super) fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            candidates: Vec::new(),
            pending_steps: Vec::new(),
        }
    }

    pub(super) fn target(&self) -> &str {
        &self.target
    }

    pub(super) fn push_current_player_candidate(&mut self) {
        self.candidates.push(GuiMediaResolutionCandidate {
            provider: GuiMediaResolutionProviderKind::Core,
            phase: GuiMediaResolutionPhase::CurrentPlayer,
            priority: GuiMediaResolutionPriority::CurrentPlayer,
            execution: GuiMediaResolutionExecution::Synchronous,
            target: GuiMediaResolutionTarget::CurrentPlayer,
        });
    }

    pub(super) fn push_user_media_candidate(
        &mut self,
        path: String,
        source: GuiUserMediaTargetResolutionSource,
    ) {
        let (provider, phase, priority) = match source {
            GuiUserMediaTargetResolutionSource::QuickLocal => (
                GuiMediaResolutionProviderKind::Core,
                GuiMediaResolutionPhase::ExactLocal,
                GuiMediaResolutionPriority::LocalExact,
            ),
            GuiUserMediaTargetResolutionSource::MediaMatchExactInventory => (
                GuiMediaResolutionProviderKind::Core,
                GuiMediaResolutionPhase::ExactLocal,
                GuiMediaResolutionPriority::LocalExact,
            ),
            GuiUserMediaTargetResolutionSource::MediaSearchIndex => (
                GuiMediaResolutionProviderKind::MediaSearch,
                GuiMediaResolutionPhase::IndexedLocal,
                GuiMediaResolutionPriority::IndexedLocal,
            ),
        };
        self.candidates.push(GuiMediaResolutionCandidate {
            provider,
            phase,
            priority,
            execution: GuiMediaResolutionExecution::Synchronous,
            target: GuiMediaResolutionTarget::LocalPath(path),
        });
    }

    pub(super) fn push_media_match_candidate(&mut self, path: String) {
        self.candidates.push(GuiMediaResolutionCandidate {
            provider: GuiMediaResolutionProviderKind::MediaMatch,
            phase: GuiMediaResolutionPhase::AlternateMatch,
            priority: GuiMediaResolutionPriority::AlternateMatch,
            execution: GuiMediaResolutionExecution::Background,
            target: GuiMediaResolutionTarget::LocalPath(path),
        });
    }

    pub(super) fn push_plex_stream_candidate(&mut self, stream_target: PlexStreamTarget) {
        self.candidates.push(GuiMediaResolutionCandidate {
            provider: GuiMediaResolutionProviderKind::Plex,
            phase: GuiMediaResolutionPhase::StreamFallback,
            priority: GuiMediaResolutionPriority::StreamFallback,
            execution: GuiMediaResolutionExecution::Background,
            target: GuiMediaResolutionTarget::PlexStream(Box::new(stream_target)),
        });
    }

    pub(super) fn record_pending_media_search(&mut self) {
        self.record_pending_background(
            GuiMediaResolutionProviderKind::MediaSearch,
            GuiMediaResolutionPhase::IndexedLocal,
            GuiMediaResolutionPriority::IndexedLocal,
        );
    }

    pub(super) fn record_pending_media_match(&mut self) {
        self.record_pending_background(
            GuiMediaResolutionProviderKind::MediaMatch,
            GuiMediaResolutionPhase::AlternateMatch,
            GuiMediaResolutionPriority::AlternateMatch,
        );
    }

    pub(super) fn record_pending_plex_stream(&mut self) {
        self.record_pending_background(
            GuiMediaResolutionProviderKind::Plex,
            GuiMediaResolutionPhase::StreamFallback,
            GuiMediaResolutionPriority::StreamFallback,
        );
    }

    pub(super) fn best_candidate(&self) -> Option<&GuiMediaResolutionCandidate> {
        self.candidates
            .iter()
            .min_by_key(|candidate| candidate.selection_key())
    }

    pub(super) fn exclude_failed_candidates(
        &mut self,
        failed_candidates: &[GuiMediaResolutionCandidate],
    ) {
        self.candidates
            .retain(|candidate| !failed_candidates.contains(candidate));
    }

    pub(super) fn decision(
        &self,
        fallback_policy: GuiMediaResolutionFallbackPolicy,
    ) -> GuiMediaResolutionDecision {
        let best_candidate = self.best_candidate();
        if matches!(
            fallback_policy,
            GuiMediaResolutionFallbackPolicy::WaitForHigherPriority
        ) {
            let candidate_priority = best_candidate.map(|candidate| candidate.priority);
            let has_blocking_pending_step = self
                .pending_steps
                .iter()
                .any(|step| candidate_priority.is_none_or(|priority| step.priority < priority));
            if has_blocking_pending_step {
                return GuiMediaResolutionDecision::WaitingForHigherPriority;
            }
        }

        match best_candidate {
            Some(candidate) => GuiMediaResolutionDecision::Ready(candidate.clone()),
            None if self.pending_steps.is_empty() => GuiMediaResolutionDecision::Exhausted,
            None => GuiMediaResolutionDecision::WaitingForHigherPriority,
        }
    }

    fn record_pending_background(
        &mut self,
        provider: GuiMediaResolutionProviderKind,
        phase: GuiMediaResolutionPhase,
        priority: GuiMediaResolutionPriority,
    ) {
        let step = GuiMediaResolutionPendingStep {
            provider,
            phase,
            priority,
            execution: GuiMediaResolutionExecution::Background,
        };
        if !self.pending_steps.contains(&step) {
            self.pending_steps.push(step);
        }
    }
}

impl GuiMediaResolutionCandidate {
    fn selection_key(
        &self,
    ) -> (
        GuiMediaResolutionPriority,
        GuiMediaResolutionProviderKind,
        GuiMediaResolutionPhase,
        GuiMediaResolutionExecution,
    ) {
        (self.priority, self.provider, self.phase, self.execution)
    }

    pub(super) fn target(&self) -> &GuiMediaResolutionTarget {
        &self.target
    }

    pub(super) fn provider_kind(&self) -> GuiMediaResolutionProviderKind {
        self.provider
    }

    pub(super) fn provider_id(&self) -> GuiMediaSourceProviderId {
        match self.provider {
            GuiMediaResolutionProviderKind::Core | GuiMediaResolutionProviderKind::MediaSearch => {
                GuiMediaSourceProviderId::local()
            }
            GuiMediaResolutionProviderKind::MediaMatch => {
                GuiMediaSourceProviderId::media_matching()
            }
            GuiMediaResolutionProviderKind::Plex => GuiMediaSourceProviderId::plex_stream(),
        }
    }

    pub(super) fn matches_loaded_target(&self, loaded_target: &str) -> bool {
        match &self.target {
            GuiMediaResolutionTarget::CurrentPlayer => false,
            GuiMediaResolutionTarget::LocalPath(path) => path == loaded_target,
            GuiMediaResolutionTarget::PlexStream(stream_target) => {
                stream_target.playback_url.as_str() == loaded_target
            }
        }
    }
}

#[cfg(test)]
mod credential_debug_tests {
    use super::*;

    #[test]
    fn media_resolution_target_candidate_and_plan_redact_tokenized_paths() {
        let secret = "https://media.example/video?access_token=resolution-canary";
        let target = GuiMediaResolutionTarget::LocalPath(secret.to_owned());
        let mut plan = GuiMediaResolutionPlan::new(secret);
        plan.push_user_media_candidate(
            secret.to_owned(),
            GuiUserMediaTargetResolutionSource::QuickLocal,
        );
        let candidate = plan
            .best_candidate()
            .expect("media-resolution candidate should exist");

        for debug in [
            format!("{target:?}"),
            format!("{candidate:?}"),
            format!("{plan:?}"),
        ] {
            assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
            assert!(!debug.contains("resolution-canary"));
        }
    }
}

#[cfg(test)]
mod decision_tests {
    use super::*;

    fn plex_stream_target() -> PlexStreamTarget {
        let playlist_uri = sorotte_plex::PlexPlaylistUri {
            machine_identifier: "machine".to_owned(),
            rating_key: "123".to_owned(),
            title: Some("Episode".to_owned()),
            file_name: Some("episode.mkv".to_owned()),
            duration_millis: None,
            size_bytes: None,
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        };
        PlexStreamTarget {
            logical_file: sorotte_player_api::LocalFileUpdate::new("episode.mkv"),
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
        }
    }

    #[test]
    fn decision_waits_for_all_higher_priority_work_before_ready_plex() {
        let mut pending_search_plan = GuiMediaResolutionPlan::new("episode.mkv");
        pending_search_plan.record_pending_media_search();
        pending_search_plan.push_plex_stream_candidate(plex_stream_target());
        assert_eq!(
            pending_search_plan.decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority),
            GuiMediaResolutionDecision::WaitingForHigherPriority
        );

        let mut pending_media_match_plan = GuiMediaResolutionPlan::new("episode.mkv");
        pending_media_match_plan.record_pending_media_match();
        pending_media_match_plan.push_plex_stream_candidate(plex_stream_target());
        assert_eq!(
            pending_media_match_plan
                .decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority),
            GuiMediaResolutionDecision::WaitingForHigherPriority
        );
        assert!(matches!(
            pending_media_match_plan
                .decision(GuiMediaResolutionFallbackPolicy::AllowReadyFallback),
            GuiMediaResolutionDecision::Ready(candidate)
                if matches!(candidate.target(), GuiMediaResolutionTarget::PlexStream(_))
        ));
    }

    #[test]
    fn decision_waits_without_candidates_and_is_exhausted_after_pending_work_settles() {
        let mut pending_plan = GuiMediaResolutionPlan::new("episode.mkv");
        pending_plan.record_pending_media_search();
        assert_eq!(
            pending_plan.decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority),
            GuiMediaResolutionDecision::WaitingForHigherPriority
        );

        let exhausted_plan = GuiMediaResolutionPlan::new("episode.mkv");
        assert_eq!(
            exhausted_plan.decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority),
            GuiMediaResolutionDecision::Exhausted
        );
    }

    #[test]
    fn decision_uses_ready_higher_priority_candidate_despite_lower_pending_work() {
        let mut plan = GuiMediaResolutionPlan::new("episode.mkv");
        plan.push_user_media_candidate(
            "C:\\media\\episode.mkv".to_owned(),
            GuiUserMediaTargetResolutionSource::QuickLocal,
        );
        plan.record_pending_media_match();
        plan.record_pending_plex_stream();

        assert!(matches!(
            plan.decision(GuiMediaResolutionFallbackPolicy::WaitForHigherPriority),
            GuiMediaResolutionDecision::Ready(candidate)
                if matches!(candidate.target(), GuiMediaResolutionTarget::LocalPath(_))
        ));
    }
}
