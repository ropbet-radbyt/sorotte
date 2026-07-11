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
pub(super) struct GuiMediaResolutionCandidate {
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
                GuiMediaResolutionProviderKind::MediaMatch,
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

    pub(super) fn has_pending_media_search_above(
        &self,
        priority: GuiMediaResolutionPriority,
    ) -> bool {
        self.pending_steps.iter().any(|step| {
            step.provider == GuiMediaResolutionProviderKind::MediaSearch && step.priority < priority
        })
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

    pub(super) fn priority(&self) -> GuiMediaResolutionPriority {
        self.priority
    }

    pub(super) fn target(&self) -> &GuiMediaResolutionTarget {
        &self.target
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
