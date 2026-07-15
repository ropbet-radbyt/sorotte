use sorotte_protocol::{
    MixedReadinessPolicy, ParticipantReadinessUpdate, ReadinessIntentRequest, RecoveryStage,
    RoomReadinessSnapshot, RoomStartGatePhase, StartGateDegradedReason, StartParticipationRole,
    TechnicalBlockCause, TechnicalPlayabilityPhase, UserReadinessIntent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessPresentationProtocol {
    Legacy,
    V2,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PendingReadinessIntentPresentation {
    pub operation_id: String,
    pub request_nonce: u64,
    pub membership_epoch: u64,
    pub desired: UserReadinessIntent,
}

impl std::fmt::Debug for PendingReadinessIntentPresentation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingReadinessIntentPresentation")
            .field("operation_id", &"<redacted>")
            .field("request_nonce", &self.request_nonce)
            .field("membership_epoch", &self.membership_epoch)
            .field("desired", &self.desired)
            .finish()
    }
}

impl From<&ReadinessIntentRequest> for PendingReadinessIntentPresentation {
    fn from(request: &ReadinessIntentRequest) -> Self {
        Self {
            operation_id: request.operation_id.clone(),
            request_nonce: request.request_nonce,
            membership_epoch: request.membership_epoch,
            desired: request.desired,
        }
    }
}

impl From<&sorotte_client_core::PendingReadinessIntent> for PendingReadinessIntentPresentation {
    fn from(pending: &sorotte_client_core::PendingReadinessIntent) -> Self {
        Self {
            operation_id: pending.operation_id().to_owned(),
            request_nonce: pending.request_nonce(),
            membership_epoch: pending.membership_epoch(),
            desired: pending.desired(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ParticipantReadinessPresentation {
    pub protocol: ReadinessPresentationProtocol,
    pub username: String,
    pub canonical_user_intent: UserReadinessIntent,
    pub technical_phase: Option<TechnicalPlayabilityPhase>,
    pub technical_reason: Option<TechnicalBlockCause>,
    pub recovery_stage: Option<RecoveryStage>,
    pub room_ready: bool,
    pub start_eligible: Option<bool>,
    pub membership_epoch: Option<u64>,
    pub room_readiness_revision: Option<u64>,
    pub user_intent_revision: Option<u64>,
    pub participation_role: Option<StartParticipationRole>,
    pub mixed_readiness_policy: Option<MixedReadinessPolicy>,
    pub start_gate_phase: Option<RoomStartGatePhase>,
    pub pending: Option<PendingReadinessIntentPresentation>,
    pub accepted_operation_id: Option<String>,
}

impl std::fmt::Debug for ParticipantReadinessPresentation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParticipantReadinessPresentation")
            .field("protocol", &self.protocol)
            .field("username", &self.username)
            .field("canonical_user_intent", &self.canonical_user_intent)
            .field("technical_phase", &self.technical_phase)
            .field("technical_reason", &self.technical_reason)
            .field("recovery_stage", &self.recovery_stage)
            .field("room_ready", &self.room_ready)
            .field("start_eligible", &self.start_eligible)
            .field("membership_epoch", &self.membership_epoch)
            .field("room_readiness_revision", &self.room_readiness_revision)
            .field("user_intent_revision", &self.user_intent_revision)
            .field("participation_role", &self.participation_role)
            .field("mixed_readiness_policy", &self.mixed_readiness_policy)
            .field("start_gate_phase", &self.start_gate_phase)
            .field("pending", &self.pending)
            .field(
                "accepted_operation_id",
                &self.accepted_operation_id.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl ParticipantReadinessPresentation {
    pub fn from_legacy(username: impl Into<String>, room_ready: bool) -> Self {
        Self {
            protocol: ReadinessPresentationProtocol::Legacy,
            username: username.into(),
            canonical_user_intent: if room_ready {
                UserReadinessIntent::Ready
            } else {
                UserReadinessIntent::NotReady
            },
            technical_phase: None,
            technical_reason: None,
            recovery_stage: None,
            room_ready,
            start_eligible: None,
            membership_epoch: None,
            room_readiness_revision: None,
            user_intent_revision: None,
            participation_role: None,
            mixed_readiness_policy: None,
            start_gate_phase: None,
            pending: None,
            accepted_operation_id: None,
        }
    }

    pub fn from_v2(
        canonical: &ParticipantReadinessUpdate,
        pending: Option<PendingReadinessIntentPresentation>,
    ) -> Self {
        Self {
            protocol: ReadinessPresentationProtocol::V2,
            username: canonical.username.clone(),
            canonical_user_intent: canonical.user_intent,
            technical_phase: Some(canonical.technical_state.phase),
            technical_reason: canonical.technical_state.reason,
            recovery_stage: canonical.technical_state.recovery,
            room_ready: canonical.room_ready,
            start_eligible: Some(canonical.start_eligible),
            membership_epoch: Some(canonical.membership_epoch),
            room_readiness_revision: Some(canonical.room_readiness_revision),
            user_intent_revision: Some(canonical.user_intent_revision),
            participation_role: Some(canonical.participation_role),
            mixed_readiness_policy: None,
            start_gate_phase: None,
            pending,
            accepted_operation_id: canonical.accepted_operation_id.clone(),
        }
    }

    pub fn with_room_snapshot(mut self, snapshot: &RoomReadinessSnapshot) -> Self {
        self.mixed_readiness_policy = Some(snapshot.mixed_readiness_policy);
        self.start_gate_phase = Some(snapshot.start_gate_phase.clone());
        self
    }

    pub fn pending_is_acknowledged(&self) -> bool {
        self.pending
            .as_ref()
            .zip(self.accepted_operation_id.as_ref())
            .is_some_and(|(pending, accepted)| pending.operation_id == *accepted)
    }

    pub fn has_unacknowledged_pending_intent(&self) -> bool {
        self.pending.is_some() && !self.pending_is_acknowledged()
    }

    pub fn displayed_user_intent(&self) -> UserReadinessIntent {
        self.pending
            .as_ref()
            .filter(|_| !self.pending_is_acknowledged())
            .map(|pending| pending.desired)
            .unwrap_or(self.canonical_user_intent)
    }

    pub fn displayed_ready(&self) -> bool {
        self.displayed_user_intent() == UserReadinessIntent::Ready
    }

    pub fn status_label(&self) -> String {
        let intent = self.displayed_user_intent();
        if self.protocol == ReadinessPresentationProtocol::Legacy {
            return if intent == UserReadinessIntent::Ready {
                "Ready".to_owned()
            } else {
                "Not Ready".to_owned()
            };
        }

        if self.technical_phase == Some(TechnicalPlayabilityPhase::TerminallyBlocked) {
            return "Not Ready — technical failure".to_owned();
        }

        let base = if intent == UserReadinessIntent::Ready {
            "Ready"
        } else {
            "Not Ready"
        };
        let Some(suffix) = self.technical_status_suffix() else {
            return base.to_owned();
        };
        format!("{base} — {suffix}")
    }

    pub fn retained_intent_label(&self) -> Option<&'static str> {
        (self.technical_phase == Some(TechnicalPlayabilityPhase::TerminallyBlocked)).then_some(
            match self.canonical_user_intent {
                UserReadinessIntent::Ready => "Ready",
                UserReadinessIntent::NotReady => "Not Ready",
            },
        )
    }

    pub fn intent_detail_label(&self) -> String {
        let canonical = intent_label(self.canonical_user_intent);
        if self.has_unacknowledged_pending_intent() {
            let pending = intent_label(self.displayed_user_intent());
            format!("pending={pending}, canonical={canonical}")
        } else {
            format!("canonical={canonical}")
        }
    }

    pub fn technical_detail_label(&self) -> String {
        let Some(phase) = self.technical_phase else {
            return "unavailable (legacy readiness)".to_owned();
        };
        let mut label = format!("phase={}", technical_phase_label(phase));
        if let Some(reason) = self.technical_reason {
            label.push_str(&format!(", reason={}", technical_reason_label(reason)));
        }
        if let Some(recovery) = self.recovery_stage {
            label.push_str(&format!(", recovery={}", recovery_stage_label(recovery)));
        }
        label
    }

    pub fn eligibility_detail_label(&self) -> String {
        format!(
            "room_ready={}, start_eligible={}",
            yes_no(self.room_ready),
            self.start_eligible.map(yes_no).unwrap_or("unknown"),
        )
    }

    /// Describes whether this participant is part of the server's automatic
    /// start cohort.  Keep the compatibility caveat explicit: a legacy or
    /// explicitly excluded participant has no generation-scoped technical
    /// readiness guarantee even though their legacy Ready value remains
    /// visible.
    pub fn participation_detail_label(&self) -> &'static str {
        match (self.participation_role, self.mixed_readiness_policy) {
            (
                Some(StartParticipationRole::ExcludedLegacy),
                Some(MixedReadinessPolicy::RequireAllMembers),
            ) => {
                "legacy participant; automatic start unavailable until every member supports readiness V2"
            }
            (
                Some(StartParticipationRole::ExcludedLegacy),
                Some(MixedReadinessPolicy::ExcludeLegacy),
            ) => "excluded legacy by compatibility policy; technical start guarantees unavailable",
            (None, Some(MixedReadinessPolicy::RequireAllMembers)) => {
                "legacy participant; automatic start unavailable until every member supports readiness V2"
            }
            (None, Some(MixedReadinessPolicy::ExcludeLegacy)) => {
                "excluded legacy by compatibility policy; technical start guarantees unavailable"
            }
            (Some(StartParticipationRole::Required), _) => "required",
            (Some(StartParticipationRole::Spectator), _) => {
                "spectator; excluded from automatic start"
            }
            (Some(StartParticipationRole::ExcludedLegacy), _) => {
                "excluded legacy; technical start guarantees unavailable"
            }
            (None, _) => "legacy participant; technical start guarantees unavailable",
        }
    }

    pub fn start_gate_detail_label(&self) -> String {
        match self.start_gate_phase.as_ref() {
            None => "unavailable (legacy readiness)".to_owned(),
            Some(RoomStartGatePhase::Inactive) => "inactive".to_owned(),
            Some(RoomStartGatePhase::WaitingForIntent { .. }) => {
                "waiting for required participants to become Ready".to_owned()
            }
            Some(RoomStartGatePhase::WaitingForTechnicalReadiness { .. }) => {
                "waiting for required participants to become technically playable".to_owned()
            }
            Some(RoomStartGatePhase::ReadyToCommit { .. }) => {
                "all readiness conditions met; awaiting server commit".to_owned()
            }
            Some(RoomStartGatePhase::Committed { .. }) => "committed by server".to_owned(),
            Some(RoomStartGatePhase::Degraded {
                reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
                ..
            }) => "automatic start unavailable: a room member does not support readiness V2"
                .to_owned(),
            Some(RoomStartGatePhase::Degraded { reason, .. }) => {
                format!(
                    "automatic start degraded: {}",
                    degraded_reason_label(*reason)
                )
            }
        }
    }

    pub fn revision_detail_label(&self) -> String {
        format!(
            "membership_epoch={}, room_revision={}, intent_revision={}",
            optional_revision(self.membership_epoch),
            optional_revision(self.room_readiness_revision),
            optional_revision(self.user_intent_revision),
        )
    }

    pub fn operation_detail_label(&self) -> String {
        let pending = presence(self.pending.is_some());
        let accepted = presence(self.accepted_operation_id.is_some());
        let correlated = self
            .pending
            .as_ref()
            .map(|_| yes_no(self.pending_is_acknowledged()))
            .unwrap_or("n/a");
        format!("pending={pending}, accepted={accepted}, correlated={correlated}")
    }

    pub fn technical_status_suffix(&self) -> Option<&'static str> {
        match self.technical_phase? {
            TechnicalPlayabilityPhase::Unknown => Some("technical state unknown"),
            TechnicalPlayabilityPhase::Preparing => Some("loading"),
            TechnicalPlayabilityPhase::Playable => None,
            TechnicalPlayabilityPhase::TemporarilyBlocked => Some(match self.technical_reason {
                Some(TechnicalBlockCause::Seeking) => "seeking",
                Some(TechnicalBlockCause::Prebuffering)
                | Some(TechnicalBlockCause::Rebuffering)
                | Some(TechnicalBlockCause::CachePause)
                | Some(TechnicalBlockCause::RoomBufferingPolicy) => "buffering",
                Some(TechnicalBlockCause::Recovery)
                | Some(TechnicalBlockCause::TransportRefresh)
                | Some(TechnicalBlockCause::AdapterReplacement) => "recovering",
                _ if self.recovery_stage.is_some() => "recovering",
                _ => "temporarily blocked",
            }),
            TechnicalPlayabilityPhase::TerminallyBlocked => Some("technical failure"),
        }
    }
}

fn degraded_reason_label(reason: StartGateDegradedReason) -> &'static str {
    match reason {
        StartGateDegradedReason::Superseded => "superseded",
        StartGateDegradedReason::ReadinessChanged => "readiness changed",
        StartGateDegradedReason::TechnicalFailure => "technical failure",
        StartGateDegradedReason::UserPaused => "user paused",
        StartGateDegradedReason::PauseOwnershipLost => "pause ownership lost",
        StartGateDegradedReason::Cancelled => "cancelled",
        StartGateDegradedReason::TimedOut => "timed out",
        StartGateDegradedReason::NoRequiredParticipants => "no required participants",
        StartGateDegradedReason::IncompatibleLegacyParticipant => "incompatible legacy participant",
    }
}

fn intent_label(intent: UserReadinessIntent) -> &'static str {
    match intent {
        UserReadinessIntent::Ready => "Ready",
        UserReadinessIntent::NotReady => "Not Ready",
    }
}

fn technical_phase_label(phase: TechnicalPlayabilityPhase) -> &'static str {
    match phase {
        TechnicalPlayabilityPhase::Unknown => "unknown",
        TechnicalPlayabilityPhase::Preparing => "preparing",
        TechnicalPlayabilityPhase::Playable => "playable",
        TechnicalPlayabilityPhase::TemporarilyBlocked => "temporarily-blocked",
        TechnicalPlayabilityPhase::TerminallyBlocked => "terminally-blocked",
    }
}

fn technical_reason_label(reason: TechnicalBlockCause) -> &'static str {
    match reason {
        TechnicalBlockCause::Loading => "loading",
        TechnicalBlockCause::Seeking => "seeking",
        TechnicalBlockCause::Prebuffering => "prebuffering",
        TechnicalBlockCause::Rebuffering => "rebuffering",
        TechnicalBlockCause::CachePause => "cache-pause",
        TechnicalBlockCause::RoomBufferingPolicy => "room-buffering-policy",
        TechnicalBlockCause::TransportRefresh => "transport-refresh",
        TechnicalBlockCause::MediaGenerationReplacement => "media-generation-replacement",
        TechnicalBlockCause::AdapterReplacement => "adapter-replacement",
        TechnicalBlockCause::Recovery => "recovery",
        TechnicalBlockCause::EndOfFile => "end-of-file",
        TechnicalBlockCause::MediaUnavailable => "media-unavailable",
        TechnicalBlockCause::MediaMappingUnavailable => "media-mapping-unavailable",
        TechnicalBlockCause::PlayerFailure => "player-failure",
        TechnicalBlockCause::AdapterFailure => "adapter-failure",
        TechnicalBlockCause::RecoveryExhausted => "recovery-exhausted",
        TechnicalBlockCause::Unknown => "unknown",
    }
}

fn recovery_stage_label(stage: RecoveryStage) -> &'static str {
    match stage {
        RecoveryStage::NotStarted => "not-started",
        RecoveryStage::Waiting => "waiting",
        RecoveryStage::Retrying => "retrying",
        RecoveryStage::ReloadingMedia => "reloading-media",
        RecoveryStage::RestartingPlayer => "restarting-player",
        RecoveryStage::ReplacingAdapter => "replacing-adapter",
    }
}

fn optional_revision(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn presence(value: bool) -> &'static str {
    if value { "present" } else { "none" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_protocol::{
        DirectReadinessSurface, ReadinessMutationSource, TechnicalPlayabilitySummary,
    };

    fn canonical(
        intent: UserReadinessIntent,
        phase: TechnicalPlayabilityPhase,
        room_ready: bool,
        start_eligible: bool,
        accepted_operation_id: Option<&str>,
    ) -> ParticipantReadinessUpdate {
        ParticipantReadinessUpdate {
            room_readiness_revision: 9,
            membership_epoch: 4,
            last_technical_report_sequence: 0,
            username: "alice".to_owned(),
            user_intent: intent,
            user_intent_revision: 7,
            user_intent_source: ReadinessMutationSource::DirectUser {
                surface: DirectReadinessSurface::GuiButton,
            },
            last_user_mutation: None,
            terminal_technical_block: None,
            technical_state: TechnicalPlayabilitySummary {
                phase,
                media_generation: Some(3),
                reason: None,
                recovery: None,
            },
            participation_role: StartParticipationRole::Required,
            room_ready,
            start_eligible,
            accepted_operation_id: accepted_operation_id.map(str::to_owned),
        }
    }

    fn pending(
        operation_id: &str,
        desired: UserReadinessIntent,
    ) -> PendingReadinessIntentPresentation {
        PendingReadinessIntentPresentation {
            operation_id: operation_id.to_owned(),
            request_nonce: 11,
            membership_epoch: 4,
            desired,
        }
    }

    #[test]
    fn v2_presentation_keeps_unrelated_pending_intent_distinct_from_canonical_boolean() {
        let presentation = ParticipantReadinessPresentation::from_v2(
            &canonical(
                UserReadinessIntent::Ready,
                TechnicalPlayabilityPhase::Playable,
                true,
                true,
                Some("older-operation"),
            ),
            Some(pending("new-operation", UserReadinessIntent::NotReady)),
        );

        assert!(presentation.has_unacknowledged_pending_intent());
        assert_eq!(
            presentation.displayed_user_intent(),
            UserReadinessIntent::NotReady
        );
        assert_eq!(presentation.status_label(), "Not Ready");
    }

    #[test]
    fn matching_operation_acknowledges_pending_intent_and_uses_canonical_state() {
        let presentation = ParticipantReadinessPresentation::from_v2(
            &canonical(
                UserReadinessIntent::Ready,
                TechnicalPlayabilityPhase::Playable,
                true,
                true,
                Some("operation-1"),
            ),
            Some(pending("operation-1", UserReadinessIntent::NotReady)),
        );

        assert!(presentation.pending_is_acknowledged());
        assert!(!presentation.has_unacknowledged_pending_intent());
        assert_eq!(
            presentation.displayed_user_intent(),
            UserReadinessIntent::Ready
        );
        assert_eq!(
            presentation.operation_detail_label(),
            "pending=present, accepted=present, correlated=yes"
        );
        assert!(
            !presentation
                .operation_detail_label()
                .contains("operation-1")
        );
    }

    #[test]
    fn v2_status_separates_transient_and_terminal_technical_state_from_intent() {
        let mut buffering = canonical(
            UserReadinessIntent::Ready,
            TechnicalPlayabilityPhase::TemporarilyBlocked,
            true,
            false,
            None,
        );
        buffering.technical_state.reason = Some(TechnicalBlockCause::Rebuffering);
        let buffering = ParticipantReadinessPresentation::from_v2(&buffering, None);
        assert_eq!(buffering.status_label(), "Ready — buffering");
        assert!(buffering.room_ready);
        assert_eq!(buffering.start_eligible, Some(false));

        let failed = ParticipantReadinessPresentation::from_v2(
            &canonical(
                UserReadinessIntent::Ready,
                TechnicalPlayabilityPhase::TerminallyBlocked,
                false,
                false,
                None,
            ),
            None,
        );
        assert_eq!(failed.status_label(), "Not Ready — technical failure");
        assert_eq!(failed.retained_intent_label(), Some("Ready"));
    }

    #[test]
    fn strict_mixed_room_presentation_explains_automatic_start_degradation() {
        let mut legacy = canonical(
            UserReadinessIntent::NotReady,
            TechnicalPlayabilityPhase::Unknown,
            false,
            false,
            None,
        );
        legacy.participation_role = StartParticipationRole::ExcludedLegacy;
        let snapshot = RoomReadinessSnapshot {
            room_readiness_revision: 9,
            media_generation: Some(3),
            start_gate_phase: RoomStartGatePhase::Degraded {
                media_generation: 3,
                reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
            },
            pause_owner: Default::default(),
            mixed_readiness_policy: MixedReadinessPolicy::RequireAllMembers,
            participants: Default::default(),
        };
        let presentation =
            ParticipantReadinessPresentation::from_v2(&legacy, None).with_room_snapshot(&snapshot);

        assert_eq!(
            presentation.participation_detail_label(),
            "legacy participant; automatic start unavailable until every member supports readiness V2"
        );
        assert_eq!(
            presentation.start_gate_detail_label(),
            "automatic start unavailable: a room member does not support readiness V2"
        );
    }

    #[test]
    fn legacy_presentation_does_not_claim_technical_start_eligibility() {
        let presentation = ParticipantReadinessPresentation::from_legacy("alice", true);
        assert_eq!(presentation.status_label(), "Ready");
        assert_eq!(presentation.technical_phase, None);
        assert_eq!(presentation.start_eligible, None);
        assert_eq!(
            presentation.participation_detail_label(),
            "legacy participant; technical start guarantees unavailable"
        );
    }

    #[test]
    fn excluded_legacy_role_makes_the_missing_technical_guarantee_explicit() {
        let mut canonical = canonical(
            UserReadinessIntent::Ready,
            TechnicalPlayabilityPhase::Preparing,
            true,
            false,
            None,
        );
        canonical.participation_role = StartParticipationRole::ExcludedLegacy;
        let presentation = ParticipantReadinessPresentation::from_v2(&canonical, None);

        assert_eq!(
            presentation.participation_detail_label(),
            "excluded legacy; technical start guarantees unavailable"
        );
    }
}
