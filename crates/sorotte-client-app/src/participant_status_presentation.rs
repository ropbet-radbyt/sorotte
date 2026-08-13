pub use sorotte_client_core::ClientParticipantStatusFreshness as ParticipantStatusFreshness;
use sorotte_client_core::ClientParticipantStatusView;
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlayerConnection, ParticipantStatusCorrelation,
    ParticipantStatusView, participant_status_buffer_evidence_is_eligible,
    participant_status_position_evidence_is_eligible,
};

/// Stable presentation boundary for one accepted participant report.
///
/// Construct this through [`ParticipantStatusReportPresentation::from_client_view`]
/// so wire evidence is sanitized before it reaches labels or accessibility.
///
/// ```compile_fail
/// use sorotte_client_app::app_boundary::participant_status::{
///     ParticipantStatusFreshness, ParticipantStatusReportPresentation,
/// };
///
/// let _ = ParticipantStatusReportPresentation {
///     status: panic!("not constructed"),
///     report_age_seconds: None,
///     freshness: ParticipantStatusFreshness::Unknown,
///     timeline_mismatch: false,
/// };
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ParticipantStatusReportPresentation {
    pub status: ParticipantStatusView,
    pub report_age_seconds: Option<f64>,
    pub freshness: ParticipantStatusFreshness,
    pub timeline_mismatch: bool,
}

impl ParticipantStatusReportPresentation {
    pub fn from_client_view(view: ClientParticipantStatusView, timeline_mismatch: bool) -> Self {
        let mut status = view.status;
        status.redact_ineligible_media_evidence();
        if status.correlation != Some(ParticipantStatusCorrelation::Exact) {
            status.room_offset_seconds = None;
        }
        let timeline_mismatch = timeline_mismatch
            || status.correlation == Some(ParticipantStatusCorrelation::Superseded);
        if view.freshness == ParticipantStatusFreshness::Stale {
            status.availability = sorotte_protocol::ParticipantStatusAvailability::Stale;
            status.playback_scope = None;
            status.phase = None;
            status.timeline_kind = None;
            status.position_seconds = None;
            status.logical_paused = None;
            status.playback_rate = None;
            status.paused_for_cache = None;
            status.cache_percent = None;
            status.buffered_ahead_seconds = None;
            status.sample_age_ms = None;
            status.position_sample_age_ms = None;
            status.room_offset_seconds = None;
        } else if timeline_mismatch {
            status.playback_scope = None;
            status.timeline_kind = None;
            status.position_seconds = None;
            status.logical_paused = None;
            status.playback_rate = None;
            status.paused_for_cache = None;
            status.cache_percent = None;
            status.buffered_ahead_seconds = None;
            status.sample_age_ms = None;
            status.position_sample_age_ms = None;
            status.room_offset_seconds = None;
        }
        Self {
            status,
            // Bucket redraw-sensitive age text conservatively. Ceil avoids a
            // delayed report being displayed as only 3.0 seconds old at the
            // fresh/delayed boundary.
            report_age_seconds: view
                .report_age_seconds
                .map(|age| (age.max(0.0) * 2.0).ceil() / 2.0),
            freshness: view.freshness,
            timeline_mismatch,
        }
    }

    pub fn phase_label(&self) -> &'static str {
        if self.freshness == ParticipantStatusFreshness::Stale {
            return "Status stale";
        }
        match self.status.player_connection {
            None => "Playback status unavailable",
            Some(ParticipantPlayerConnection::Unavailable) => "Player unavailable",
            Some(ParticipantPlayerConnection::Starting) => "Player starting",
            Some(ParticipantPlayerConnection::Disconnected) => "Player disconnected",
            Some(ParticipantPlayerConnection::Failed) => "Player failed",
            Some(ParticipantPlayerConnection::Connected) => participant_playback_phase_label(
                self.status
                    .phase
                    .unwrap_or(ParticipantPlaybackPhase::Unknown),
            ),
            Some(_) => "Playback status unavailable",
        }
    }

    fn has_timeline_mismatch(&self) -> bool {
        self.timeline_mismatch
            || self.status.correlation == Some(ParticipantStatusCorrelation::Superseded)
    }

    fn position_evidence_is_eligible(&self) -> bool {
        self.status
            .player_connection
            .zip(self.status.phase)
            .is_some_and(|(connection, phase)| {
                participant_status_position_evidence_is_eligible(connection, phase)
            })
    }

    fn buffer_evidence_is_eligible(&self) -> bool {
        self.status
            .player_connection
            .zip(self.status.phase)
            .is_some_and(|(connection, phase)| {
                participant_status_buffer_evidence_is_eligible(connection, phase)
            })
    }

    pub fn connection_label(&self) -> &'static str {
        match self.status.player_connection {
            None | Some(ParticipantPlayerConnection::Unavailable) => "unavailable",
            Some(ParticipantPlayerConnection::Starting) => "starting",
            Some(ParticipantPlayerConnection::Connected) => "connected",
            Some(ParticipantPlayerConnection::Disconnected) => "disconnected",
            Some(ParticipantPlayerConnection::Failed) => "failed",
            Some(_) => "unavailable",
        }
    }

    pub fn position_label(&self) -> String {
        if self.freshness == ParticipantStatusFreshness::Stale
            || self.has_timeline_mismatch()
            || !self.position_evidence_is_eligible()
        {
            return "Position unavailable".to_owned();
        }
        self.status
            .position_seconds
            .filter(|position| position.is_finite() && *position >= 0.0)
            .map(format_participant_status_timestamp)
            .unwrap_or_else(|| "Position unavailable".to_owned())
    }

    pub fn sync_label(&self) -> String {
        if self.has_timeline_mismatch() {
            return match self.status.correlation {
                Some(ParticipantStatusCorrelation::Uncorrelated) => {
                    "Playback scope not yet correlated".to_owned()
                }
                _ => "Different playback scope".to_owned(),
            };
        }
        if self.freshness != ParticipantStatusFreshness::Fresh {
            return "Offset unavailable".to_owned();
        }
        if self.status.correlation != Some(ParticipantStatusCorrelation::Exact) {
            return "Offset unavailable".to_owned();
        }
        if !self.position_evidence_is_eligible() {
            return "Offset unavailable".to_owned();
        }
        let Some(offset) = self
            .status
            .room_offset_seconds
            .filter(|offset| offset.is_finite())
        else {
            return "Offset unavailable".to_owned();
        };
        if offset.abs() < 0.05 {
            "In sync with room".to_owned()
        } else if offset.is_sign_positive() {
            format!("{:.1} s ahead", offset.abs())
        } else {
            format!("{:.1} s behind", offset.abs())
        }
    }

    pub fn offset_label(&self) -> String {
        self.sync_label()
    }

    pub fn buffer_label(&self) -> String {
        if self.freshness == ParticipantStatusFreshness::Stale
            || self.has_timeline_mismatch()
            || !self.buffer_evidence_is_eligible()
        {
            return "Buffer status unavailable".to_owned();
        }
        match (
            self.status.buffered_ahead_seconds,
            self.status.cache_percent,
        ) {
            (Some(buffered), Some(refill)) => {
                format!("{buffered:.1} s buffered · cache refill {refill:.0}%")
            }
            (Some(buffered), None) => format!("{buffered:.1} s buffered"),
            (None, Some(refill)) => format!("Cache refill {refill:.0}%"),
            (None, None) => "Buffer status unavailable".to_owned(),
        }
    }

    pub fn freshness_label(&self) -> String {
        self.report_age_seconds.map_or_else(
            || participant_status_freshness_label(self.freshness).to_owned(),
            |age| {
                format!(
                    "{} · {:.1} s old",
                    participant_status_freshness_label(self.freshness),
                    age.max(0.0)
                )
            },
        )
    }

    pub fn headline_label(&self) -> String {
        if self.freshness == ParticipantStatusFreshness::Stale {
            return self.report_age_seconds.map_or_else(
                || "Status stale".to_owned(),
                |age| format!("Status stale · last update {:.1} s ago", age.max(0.0)),
            );
        }
        let mut parts = vec![self.phase_label().to_owned()];
        if self.position_evidence_is_eligible()
            && self.status.position_seconds.is_some()
            && !self.has_timeline_mismatch()
        {
            parts.push(self.position_label());
        }
        if self.has_timeline_mismatch()
            || (self.position_evidence_is_eligible() && self.status.position_seconds.is_some())
        {
            parts.push(self.sync_label());
        }
        if self.buffer_evidence_is_eligible()
            && !self.has_timeline_mismatch()
            && (self.status.buffered_ahead_seconds.is_some() || self.status.cache_percent.is_some())
        {
            parts.push(self.buffer_label());
        }
        parts.push(participant_status_freshness_label(self.freshness).to_owned());
        parts.join(" · ")
    }

    pub fn compact_label(&self) -> String {
        self.headline_label()
    }

    pub fn detail_label(&self) -> String {
        let stale = self.freshness == ParticipantStatusFreshness::Stale;
        let precise_unavailable =
            stale || self.has_timeline_mismatch() || !self.position_evidence_is_eligible();
        let terminal_player = matches!(
            self.status.player_connection,
            None | Some(
                ParticipantPlayerConnection::Unavailable
                    | ParticipantPlayerConnection::Disconnected
                    | ParticipantPlayerConnection::Failed
            )
        );
        let player_detail = if stale {
            format!("Last reported player: {}", self.connection_label())
        } else {
            format!("Player: {}", self.connection_label())
        };
        let playback_detail = if stale {
            "Playback evidence: unavailable (status stale)".to_owned()
        } else if terminal_player {
            self.status.phase.map_or_else(
                || "Playback evidence: unavailable".to_owned(),
                |phase| {
                    format!(
                        "Last reported playback: {}",
                        participant_playback_phase_label(phase)
                    )
                },
            )
        } else {
            format!("Playback: {}", self.phase_label())
        };
        let timestamp_detail = if stale {
            "Timestamp: Position unavailable".to_owned()
        } else if terminal_player {
            format!("Last reported timestamp: {}", self.position_label())
        } else {
            format!("Timestamp: {}", self.position_label())
        };
        let offset_detail = if stale {
            "Room offset: Offset unavailable".to_owned()
        } else if terminal_player {
            format!("Last reported room offset: {}", self.sync_label())
        } else {
            format!("Room offset: {}", self.sync_label())
        };
        let buffer_detail = if stale {
            "Buffer: Buffer status unavailable".to_owned()
        } else if terminal_player {
            format!("Last reported buffer: {}", self.buffer_label())
        } else {
            format!("Buffer: {}", self.buffer_label())
        };
        let logical_pause = if precise_unavailable {
            "unavailable"
        } else {
            self.status
                .logical_paused
                .map(|paused| if paused { "yes" } else { "no" })
                .unwrap_or("unavailable")
        };
        let logical_pause_detail = if terminal_player && !stale {
            format!("Last reported logical pause: {logical_pause}")
        } else {
            format!("Logical pause: {logical_pause}")
        };
        let mut details = vec![
            format!("Sorotte connection: {}", self.freshness_label()),
            player_detail,
            playback_detail,
            timestamp_detail,
            offset_detail,
            buffer_detail,
            logical_pause_detail,
        ];
        if !precise_unavailable && let Some(rate) = self.status.playback_rate {
            let label = if terminal_player {
                "Last reported playback rate"
            } else {
                "Playback rate"
            };
            details.push(format!("{label}: {rate:.2}×"));
        }
        if !precise_unavailable && let Some(scope) = self.status.playback_scope {
            let media_generation_label = if terminal_player {
                "Last reported media generation"
            } else {
                "Media generation"
            };
            let room_revision_label = if terminal_player {
                "Last reported room revision"
            } else {
                "Room revision"
            };
            let transport_revision_label = if terminal_player {
                "Last reported transport revision"
            } else {
                "Transport revision"
            };
            details.push(format!(
                "{media_generation_label}: {}",
                scope.media_generation
            ));
            if let Some(revision) = scope.state_revision {
                details.push(format!("{room_revision_label}: {revision}"));
            }
            if let Some(revision) = scope.transport_revision {
                details.push(format!("{transport_revision_label}: {revision}"));
            }
        }
        if !precise_unavailable && let Some(sample_age_ms) = self.status.sample_age_ms {
            let label = if terminal_player {
                "Last reported underlying sample age"
            } else {
                "Underlying sample age"
            };
            details.push(format!("{label}: {sample_age_ms} ms"));
        }
        details.join("\n")
    }
}

fn participant_playback_phase_label(phase: ParticipantPlaybackPhase) -> &'static str {
    match phase {
        ParticipantPlaybackPhase::Unknown => "Playback state unknown",
        ParticipantPlaybackPhase::Empty => "No media",
        ParticipantPlaybackPhase::Loading => "Loading",
        ParticipantPlaybackPhase::Prebuffering => "Prebuffering",
        ParticipantPlaybackPhase::ReadyPaused => "Ready · paused",
        ParticipantPlaybackPhase::Playing => "Playing",
        ParticipantPlaybackPhase::Rebuffering => "Rebuffering",
        ParticipantPlaybackPhase::Seeking => "Seeking",
        ParticipantPlaybackPhase::Ended => "Ended",
        ParticipantPlaybackPhase::Failed => "Playback failed",
        _ => "Playback state unknown",
    }
}

/// Extensible top-level participant-status presentation.
///
/// Downstream matches must retain a wildcard so future additive states do not
/// become a source-compatibility break.
///
/// ```compile_fail
/// use sorotte_client_app::app_boundary::participant_status::ParticipantStatusPresentation;
///
/// fn label(value: ParticipantStatusPresentation) -> &'static str {
///     match value {
///         ParticipantStatusPresentation::Unavailable => "unavailable",
///         ParticipantStatusPresentation::LegacyClient => "legacy",
///         ParticipantStatusPresentation::WaitingForFirstReport => "waiting",
///         ParticipantStatusPresentation::Report(_) => "report",
///     }
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ParticipantStatusPresentation {
    #[default]
    Unavailable,
    LegacyClient,
    WaitingForFirstReport,
    Report(ParticipantStatusReportPresentation),
}

impl ParticipantStatusPresentation {
    pub fn headline_label(&self) -> String {
        match self {
            Self::Unavailable => "Status unavailable".to_owned(),
            Self::LegacyClient => "Status unavailable · legacy client".to_owned(),
            Self::WaitingForFirstReport => "Waiting for first status report".to_owned(),
            Self::Report(report) => report.headline_label(),
        }
    }

    pub fn compact_label(&self) -> String {
        self.headline_label()
    }

    pub fn connection_label(&self) -> String {
        match self {
            Self::Unavailable => "unavailable".to_owned(),
            Self::LegacyClient => "unavailable (legacy client)".to_owned(),
            Self::WaitingForFirstReport => "waiting for first report".to_owned(),
            Self::Report(report) => report.connection_label().to_owned(),
        }
    }

    pub fn phase_label(&self) -> String {
        match self {
            Self::Unavailable => "Status unavailable".to_owned(),
            Self::LegacyClient => "Status unavailable (legacy client)".to_owned(),
            Self::WaitingForFirstReport => "Waiting for first status report".to_owned(),
            Self::Report(report) => report.phase_label().to_owned(),
        }
    }

    pub fn position_label(&self) -> String {
        match self {
            Self::Report(report) => report.position_label(),
            _ => "Position unavailable".to_owned(),
        }
    }

    pub fn sync_label(&self) -> String {
        match self {
            Self::Report(report) => report.sync_label(),
            _ => "Offset unavailable".to_owned(),
        }
    }

    pub fn offset_label(&self) -> String {
        self.sync_label()
    }

    pub fn buffer_label(&self) -> String {
        match self {
            Self::Report(report) => report.buffer_label(),
            _ => "Buffer status unavailable".to_owned(),
        }
    }

    pub fn freshness_label(&self) -> String {
        match self {
            Self::Unavailable => "Heartbeat unavailable".to_owned(),
            Self::LegacyClient => "Heartbeat unavailable (legacy client)".to_owned(),
            Self::WaitingForFirstReport => "Waiting for first heartbeat".to_owned(),
            Self::Report(report) => report.freshness_label(),
        }
    }

    pub fn detail_label(&self) -> String {
        match self {
            Self::Unavailable => "Participant status is unavailable.".to_owned(),
            Self::LegacyClient => "Legacy client · detailed status unavailable.".to_owned(),
            Self::WaitingForFirstReport => "Awaiting first player report.".to_owned(),
            Self::Report(report) => report.detail_label(),
        }
    }
}

fn participant_status_freshness_label(freshness: ParticipantStatusFreshness) -> &'static str {
    match freshness {
        ParticipantStatusFreshness::Unknown => "freshness unknown",
        ParticipantStatusFreshness::Fresh => "fresh",
        ParticipantStatusFreshness::Delayed => "delayed",
        ParticipantStatusFreshness::Stale => "stale",
        _ => "freshness unknown",
    }
}

pub fn format_participant_status_timestamp(seconds: f64) -> String {
    let total_tenths = (seconds.max(0.0) * 10.0).round() as u64;
    let hours = total_tenths / 36_000;
    let minutes = (total_tenths / 600) % 60;
    let whole_seconds = (total_tenths / 10) % 60;
    let tenths = total_tenths % 10;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{whole_seconds:02}.{tenths}")
    } else {
        format!("{minutes:02}:{whole_seconds:02}.{tenths}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_protocol::{
        ParticipantPlaybackScope, ParticipantStatusAvailability, ParticipantTimelineKind,
    };

    fn report_presentation() -> ParticipantStatusReportPresentation {
        let mut status = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
        status.correlation = Some(ParticipantStatusCorrelation::Exact);
        status.playback_scope = Some(ParticipantPlaybackScope::new(7).with_state_revision(19));
        status.player_connection = Some(ParticipantPlayerConnection::Connected);
        status.phase = Some(ParticipantPlaybackPhase::Rebuffering);
        status.timeline_kind = Some(ParticipantTimelineKind::Vod);
        status.position_seconds = Some(751.2);
        status.logical_paused = Some(false);
        status.playback_rate = Some(1.0);
        status.paused_for_cache = Some(true);
        status.buffered_ahead_seconds = Some(0.4);
        status.cache_percent = Some(20.0);
        status.report_age_ms = Some(340);
        status.room_offset_seconds = Some(-3.83);
        ParticipantStatusReportPresentation {
            status,
            report_age_seconds: Some(0.34),
            freshness: ParticipantStatusFreshness::Fresh,
            timeline_mismatch: false,
        }
    }

    #[test]
    fn report_presentation_owns_consistent_headline_and_detail_labels() {
        let report = report_presentation();
        assert_eq!(
            report.headline_label(),
            "Rebuffering · 12:31.2 · 3.8 s behind · 0.4 s buffered · cache refill 20% · fresh"
        );
        let detail = report.detail_label();
        for expected in [
            "Sorotte connection: fresh · 0.3 s old",
            "Player: connected",
            "Playback: Rebuffering",
            "Timestamp: 12:31.2",
            "Room offset: 3.8 s behind",
            "Playback rate: 1.00×",
            "Media generation: 7",
            "Room revision: 19",
        ] {
            assert!(detail.contains(expected), "missing {expected:?}: {detail}");
        }
    }

    #[test]
    fn stale_priority_suppresses_old_playback_evidence_from_the_headline() {
        let mut report = report_presentation();
        report.freshness = ParticipantStatusFreshness::Stale;
        report.report_age_seconds = Some(12.0);
        assert_eq!(
            report.headline_label(),
            "Status stale · last update 12.0 s ago"
        );
        assert_eq!(report.phase_label(), "Status stale");
        let detail = report.detail_label();
        assert!(detail.contains("Last reported player: connected"));
        assert!(detail.contains("Playback evidence: unavailable (status stale)"));
        assert!(!detail.contains("Playback: Rebuffering"));
    }

    #[test]
    fn stale_factory_redacts_detail_fields_even_for_inconsistent_callers() {
        let mut status = report_presentation().status;
        status.position_sample_age_ms = Some(125);
        let mut view = ClientParticipantStatusView::from_wire(status);
        view.report_age_seconds = Some(12.0);
        view.freshness = ParticipantStatusFreshness::Stale;
        let report = ParticipantStatusReportPresentation::from_client_view(view, false);
        assert_eq!(report.status.phase, None);
        assert_eq!(report.status.position_seconds, None);
        assert_eq!(report.status.buffered_ahead_seconds, None);
        assert_eq!(report.status.playback_rate, None);
        assert_eq!(report.status.position_sample_age_ms, None);
        assert_eq!(report.status.playback_scope, None);
        let detail = report.detail_label();
        assert!(detail.contains("Last reported player: connected"));
        assert!(detail.contains("Playback evidence: unavailable (status stale)"));
        assert!(detail.contains("Timestamp: Position unavailable"));
        assert!(detail.contains("Buffer: Buffer status unavailable"));
        assert!(!detail.contains("Playback rate:"));
        assert!(!detail.contains("Media generation:"));
    }

    #[test]
    fn exact_factory_preserves_a_valid_room_offset() {
        let mut status = report_presentation().status;
        status.sample_age_ms = Some(0);
        status.position_sample_age_ms = Some(0);
        let mut view = ClientParticipantStatusView::from_wire(status);
        view.report_age_seconds = Some(0.34);
        view.freshness = ParticipantStatusFreshness::Fresh;

        let presentation = ParticipantStatusReportPresentation::from_client_view(view, false);

        assert_eq!(presentation.status.room_offset_seconds, Some(-3.83));
        assert_eq!(presentation.sync_label(), "3.8 s behind");
        assert!(presentation.headline_label().contains("3.8 s behind"));
    }

    #[test]
    fn mismatched_scope_preserves_coarse_truth_but_redacts_media_evidence() {
        let mut status = report_presentation().status;
        status.correlation = Some(ParticipantStatusCorrelation::Superseded);
        status.sample_age_ms = Some(250);
        let mut view = ClientParticipantStatusView::from_wire(status);
        view.report_age_seconds = Some(0.34);
        view.freshness = ParticipantStatusFreshness::Fresh;
        let presentation = ParticipantStatusReportPresentation::from_client_view(view, false);

        assert!(presentation.timeline_mismatch);
        assert_eq!(
            presentation.status.player_connection,
            Some(ParticipantPlayerConnection::Connected)
        );
        assert_eq!(
            presentation.status.phase,
            Some(ParticipantPlaybackPhase::Rebuffering)
        );
        assert_eq!(presentation.status.playback_scope, None);
        assert_eq!(presentation.status.position_seconds, None);
        assert_eq!(presentation.status.logical_paused, None);
        assert_eq!(presentation.status.playback_rate, None);
        assert_eq!(presentation.status.paused_for_cache, None);
        assert_eq!(presentation.status.cache_percent, None);
        assert_eq!(presentation.status.buffered_ahead_seconds, None);
        assert_eq!(presentation.status.sample_age_ms, None);
        assert_eq!(presentation.status.room_offset_seconds, None);
        assert_eq!(presentation.position_label(), "Position unavailable");
        assert_eq!(presentation.buffer_label(), "Buffer status unavailable");
        assert_eq!(presentation.sync_label(), "Different playback scope");
    }

    #[test]
    fn public_field_mutation_cannot_reintroduce_mismatched_precision_in_labels() {
        let mut presentation = report_presentation();
        presentation.status.correlation = Some(ParticipantStatusCorrelation::Superseded);
        presentation.status.sample_age_ms = Some(10);
        presentation.status.position_sample_age_ms = Some(10);

        assert_eq!(presentation.position_label(), "Position unavailable");
        assert_eq!(presentation.buffer_label(), "Buffer status unavailable");
        assert_eq!(presentation.sync_label(), "Different playback scope");
        let headline = presentation.headline_label();
        assert!(!headline.contains("12:31.2"), "{headline}");
        assert!(!headline.contains("buffered"), "{headline}");
        let detail = presentation.detail_label();
        assert!(
            detail.contains("Timestamp: Position unavailable"),
            "{detail}"
        );
        assert!(
            detail.contains("Buffer: Buffer status unavailable"),
            "{detail}"
        );
        assert!(!detail.contains("Playback rate:"), "{detail}");
        assert!(!detail.contains("Media generation:"), "{detail}");
        assert!(!detail.contains("Underlying sample age:"), "{detail}");

        presentation.status.correlation = None;
        presentation.timeline_mismatch = false;
        presentation.status.room_offset_seconds = Some(-3.83);
        assert_eq!(presentation.sync_label(), "Offset unavailable");
        assert!(!presentation.headline_label().contains("behind"));
    }

    #[test]
    fn uncorrelated_legacy_position_remains_visible_without_a_precise_offset() {
        let mut status = report_presentation().status;
        status.correlation = Some(ParticipantStatusCorrelation::Uncorrelated);
        status.player_connection = Some(ParticipantPlayerConnection::Connected);
        status.phase = Some(ParticipantPlaybackPhase::Playing);
        status.paused_for_cache = Some(false);
        status.sample_age_ms = Some(0);
        status.position_sample_age_ms = Some(0);
        status.room_offset_seconds = Some(-3.83);
        let presentation = ParticipantStatusReportPresentation::from_client_view(
            ClientParticipantStatusView::from_wire(status),
            false,
        );

        assert!(!presentation.timeline_mismatch);
        assert_eq!(presentation.position_label(), "12:31.2");
        assert_eq!(presentation.status.room_offset_seconds, None);
        assert_eq!(presentation.sync_label(), "Offset unavailable");
        assert!(presentation.headline_label().contains("12:31.2"));
        assert!(presentation.headline_label().contains("Offset unavailable"));
        assert!(!presentation.headline_label().contains("behind"));
    }

    #[test]
    fn headline_explains_offset_availability_whenever_position_is_visible() {
        let mut exact_without_offset = report_presentation();
        exact_without_offset.status.room_offset_seconds = None;
        assert!(
            exact_without_offset
                .headline_label()
                .contains("Offset unavailable")
        );

        let mut legacy_without_position = report_presentation();
        legacy_without_position.status.correlation = None;
        legacy_without_position.status.position_seconds = None;
        legacy_without_position.status.room_offset_seconds = None;
        assert!(
            !legacy_without_position
                .headline_label()
                .contains("Offset unavailable")
        );

        let mut legacy_with_position = report_presentation();
        legacy_with_position.status.correlation = None;
        legacy_with_position.status.room_offset_seconds = None;
        assert!(
            legacy_with_position
                .headline_label()
                .contains("Offset unavailable")
        );
    }

    #[test]
    fn timestamp_rounding_carries_across_minute_and_hour_boundaries() {
        assert_eq!(format_participant_status_timestamp(59.96), "01:00.0");
        assert_eq!(format_participant_status_timestamp(3_599.96), "01:00:00.0");
    }

    #[test]
    fn age_buckets_never_understate_freshness_boundary_crossings() {
        for (age, freshness, expected) in [
            (0.18, ParticipantStatusFreshness::Fresh, "fresh · 0.5 s old"),
            (
                3.01,
                ParticipantStatusFreshness::Delayed,
                "delayed · 3.5 s old",
            ),
            (
                10.01,
                ParticipantStatusFreshness::Stale,
                "stale · 10.5 s old",
            ),
        ] {
            let mut view = ClientParticipantStatusView::from_wire(report_presentation().status);
            view.report_age_seconds = Some(age);
            view.freshness = freshness;
            let report = ParticipantStatusReportPresentation::from_client_view(view, false);
            assert_eq!(report.freshness_label(), expected);
        }
    }

    #[test]
    fn unsupported_awaiting_and_player_connection_states_remain_distinct() {
        assert_eq!(
            ParticipantStatusPresentation::LegacyClient.headline_label(),
            "Status unavailable · legacy client"
        );
        assert_eq!(
            ParticipantStatusPresentation::WaitingForFirstReport.headline_label(),
            "Waiting for first status report"
        );
        let mut report = report_presentation();
        report.status.player_connection = Some(ParticipantPlayerConnection::Disconnected);
        assert_eq!(report.headline_label(), "Player disconnected · fresh");
        assert_eq!(report.position_label(), "Position unavailable");
        assert_eq!(report.buffer_label(), "Buffer status unavailable");
        let detail = report.detail_label();
        assert!(detail.contains("Player: disconnected"));
        assert!(detail.contains("Last reported playback: Rebuffering"));
        assert!(!detail.contains("Playback: Player disconnected"));
        assert!(!report.headline_label().contains("12:31.2"));
        assert!(!report.headline_label().contains("buffered"));
    }

    #[test]
    fn lifecycle_and_phase_priority_suppresses_incompatible_compact_precision() {
        for (connection, phase, expected) in [
            (
                ParticipantPlayerConnection::Connected,
                ParticipantPlaybackPhase::Empty,
                "No media · fresh",
            ),
            (
                ParticipantPlayerConnection::Connected,
                ParticipantPlaybackPhase::Loading,
                "Loading · fresh",
            ),
            (
                ParticipantPlayerConnection::Connected,
                ParticipantPlaybackPhase::Seeking,
                "Seeking · fresh",
            ),
            (
                ParticipantPlayerConnection::Connected,
                ParticipantPlaybackPhase::Failed,
                "Playback failed · fresh",
            ),
            (
                ParticipantPlayerConnection::Starting,
                ParticipantPlaybackPhase::Loading,
                "Player starting · fresh",
            ),
            (
                ParticipantPlayerConnection::Failed,
                ParticipantPlaybackPhase::Failed,
                "Player failed · fresh",
            ),
        ] {
            let mut report = report_presentation();
            report.status.player_connection = Some(connection);
            report.status.phase = Some(phase);
            assert_eq!(report.headline_label(), expected);
            assert_eq!(report.position_label(), "Position unavailable");
            assert_eq!(report.buffer_label(), "Buffer status unavailable");
        }

        for (buffered_ahead_seconds, cache_percent, expected) in [
            (Some(8.0), None, "8.0 s buffered"),
            (None, Some(75.0), "Cache refill 75%"),
        ] {
            let mut report = report_presentation();
            report.status.buffered_ahead_seconds = buffered_ahead_seconds;
            report.status.cache_percent = cache_percent;
            assert!(
                report.headline_label().contains(expected),
                "a single eligible buffer observation must remain visible"
            );
        }
    }
}
