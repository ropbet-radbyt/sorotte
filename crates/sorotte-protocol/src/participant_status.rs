use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hello feature and nested State extension key for Sorotte's additive,
/// advisory participant-player telemetry channel.
pub const SOROTTE_PARTICIPANT_STATUS_V1: &str = "sorotteParticipantStatusV1";

/// Inclusive numeric limits for optional participant-status report fields.
/// Clients omit observations outside these bounds and servers reject them.
pub const PARTICIPANT_STATUS_MAX_POSITION_SECONDS: f64 = 31_536_000.0;
pub const PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS: f64 = 86_400.0;
pub const PARTICIPANT_STATUS_MIN_PLAYBACK_RATE: f64 = 0.05;
pub const PARTICIPANT_STATUS_MAX_PLAYBACK_RATE: f64 = 16.0;
pub const PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS: u64 = 60_000;

/// Whether a participant's Sorotte client can currently reach its player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantPlayerConnection {
    Unavailable,
    Starting,
    Connected,
    Disconnected,
    Failed,
}

/// Stable, privacy-safe projection of the participant's coarse playback phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantPlaybackPhase {
    Empty,
    Loading,
    Prebuffering,
    ReadyPaused,
    Playing,
    Rebuffering,
    Seeking,
    Ended,
    Failed,
    Unknown,
}

/// Whether position-derived evidence is meaningful for this lifecycle/phase.
/// Loading, seeking, and terminal player states may retain coarse truth, but
/// must not expose a precision-looking timestamp from an older observation.
pub fn participant_status_position_evidence_is_eligible(
    player_connection: ParticipantPlayerConnection,
    phase: ParticipantPlaybackPhase,
) -> bool {
    player_connection == ParticipantPlayerConnection::Connected
        && matches!(
            phase,
            ParticipantPlaybackPhase::ReadyPaused
                | ParticipantPlaybackPhase::Playing
                | ParticipantPlaybackPhase::Rebuffering
                | ParticipantPlaybackPhase::Ended
        )
}

/// Whether cache/buffer evidence is meaningful for this lifecycle/phase.
pub fn participant_status_buffer_evidence_is_eligible(
    player_connection: ParticipantPlayerConnection,
    phase: ParticipantPlaybackPhase,
) -> bool {
    player_connection == ParticipantPlayerConnection::Connected
        && matches!(
            phase,
            ParticipantPlaybackPhase::Prebuffering
                | ParticipantPlaybackPhase::ReadyPaused
                | ParticipantPlaybackPhase::Playing
                | ParticipantPlaybackPhase::Rebuffering
        )
}

/// Timeline class used to prevent misleading VOD-style drift for live media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantTimelineKind {
    Vod,
    Live,
    Unknown,
}

/// Optional authoritative scope that correlates a report to the room media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub struct ParticipantPlaybackScope {
    pub media_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_revision: Option<u64>,
    /// Monotonic room transport-authority fence. This changes for canonical
    /// seek/pause authority even when the media/barrier generation does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_revision: Option<u64>,
}

impl ParticipantPlaybackScope {
    pub fn new(media_generation: u64) -> Self {
        Self {
            media_generation,
            state_revision: None,
            transport_revision: None,
        }
    }

    pub fn with_state_revision(mut self, state_revision: u64) -> Self {
        self.state_revision = Some(state_revision);
        self
    }

    pub fn with_transport_revision(mut self, transport_revision: u64) -> Self {
        self.transport_revision = Some(transport_revision);
        self
    }
}

/// Connection-scoped status observation. Identity and room are deliberately
/// absent: the server derives both from the authenticated session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub struct ParticipantStatusReport {
    /// Strictly increasing and non-zero within one protocol connection.
    pub report_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_scope: Option<ParticipantPlaybackScope>,
    pub player_connection: ParticipantPlayerConnection,
    pub phase: ParticipantPlaybackPhase,
    pub timeline_kind: ParticipantTimelineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_for_cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered_ahead_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_age_ms: Option<u64>,
    /// Age of the retained position sample specifically. Sparse player
    /// telemetry can refresh another field without refreshing position, so
    /// position projection must not infer this value from the report-wide
    /// oldest-evidence age.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_sample_age_ms: Option<u64>,
}

impl ParticipantStatusReport {
    pub fn new(
        report_sequence: u64,
        player_connection: ParticipantPlayerConnection,
        phase: ParticipantPlaybackPhase,
    ) -> Self {
        Self {
            report_sequence,
            playback_scope: None,
            player_connection,
            phase,
            timeline_kind: ParticipantTimelineKind::Unknown,
            position_seconds: None,
            logical_paused: None,
            playback_rate: None,
            paused_for_cache: None,
            cache_percent: None,
            buffered_ahead_seconds: None,
            sample_age_ms: None,
            position_sample_age_ms: None,
        }
    }

    pub fn with_playback_scope(mut self, playback_scope: ParticipantPlaybackScope) -> Self {
        self.playback_scope = Some(playback_scope);
        self
    }

    pub fn with_timeline_kind(mut self, timeline_kind: ParticipantTimelineKind) -> Self {
        self.timeline_kind = timeline_kind;
        self
    }

    pub fn with_position_seconds(mut self, position_seconds: f64) -> Self {
        self.position_seconds = Some(position_seconds);
        self
    }

    pub fn with_logical_paused(mut self, logical_paused: bool) -> Self {
        self.logical_paused = Some(logical_paused);
        self
    }

    pub fn with_playback_rate(mut self, playback_rate: f64) -> Self {
        self.playback_rate = Some(playback_rate);
        self
    }

    pub fn with_paused_for_cache(mut self, paused_for_cache: bool) -> Self {
        self.paused_for_cache = Some(paused_for_cache);
        self
    }

    pub fn with_cache_percent(mut self, cache_percent: f64) -> Self {
        self.cache_percent = Some(cache_percent);
        self
    }

    pub fn with_buffered_ahead_seconds(mut self, buffered_ahead_seconds: f64) -> Self {
        self.buffered_ahead_seconds = Some(buffered_ahead_seconds);
        self
    }

    pub fn with_sample_age_ms(mut self, sample_age_ms: u64) -> Self {
        self.sample_age_ms = Some(sample_age_ms);
        self
    }

    pub fn with_position_sample_age_ms(mut self, position_sample_age_ms: u64) -> Self {
        self.position_sample_age_ms = Some(position_sample_age_ms);
        self
    }

    /// Removes schema-valid but lifecycle-incompatible precision. This is a
    /// sanitization step, not report rejection: coarse advisory truth remains.
    pub fn redact_ineligible_media_evidence(&mut self) {
        let position_eligible =
            participant_status_position_evidence_is_eligible(self.player_connection, self.phase);
        let buffer_eligible =
            participant_status_buffer_evidence_is_eligible(self.player_connection, self.phase);
        if !position_eligible {
            self.timeline_kind = ParticipantTimelineKind::Unknown;
            self.position_seconds = None;
            self.logical_paused = None;
            self.playback_rate = None;
            self.position_sample_age_ms = None;
        }
        if !buffer_eligible {
            self.paused_for_cache = None;
            self.cache_percent = None;
            self.buffered_ahead_seconds = None;
        }
        if !position_eligible && !buffer_eligible {
            self.sample_age_ms = None;
        }
    }
}

/// Server-derived freshness or capability state for one room participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantStatusAvailability {
    Fresh,
    Delayed,
    Stale,
    AwaitingReport,
    Unsupported,
    /// The server deliberately omitted a population-sized snapshot because
    /// even its compact representation could not fit the negotiated frame.
    Unavailable,
}

/// Correlation of one retained report to the server's current room scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantStatusCorrelation {
    Exact,
    Uncorrelated,
    Superseded,
}

/// Server-owned projection of one participant's latest accepted report.
/// `room_offset_seconds` is never supplied by the reporting client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub struct ParticipantStatusView {
    pub availability: ParticipantStatusAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<ParticipantStatusCorrelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_scope: Option<ParticipantPlaybackScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_connection: Option<ParticipantPlayerConnection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ParticipantPlaybackPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_kind: Option<ParticipantTimelineKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_for_cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffered_ahead_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_sample_age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_age_ms: Option<u64>,
    /// Positive means ahead of the authoritative room timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_offset_seconds: Option<f64>,
}

impl ParticipantStatusView {
    pub fn new(availability: ParticipantStatusAvailability) -> Self {
        Self {
            availability,
            correlation: None,
            playback_scope: None,
            player_connection: None,
            phase: None,
            timeline_kind: None,
            position_seconds: None,
            logical_paused: None,
            playback_rate: None,
            paused_for_cache: None,
            cache_percent: None,
            buffered_ahead_seconds: None,
            sample_age_ms: None,
            position_sample_age_ms: None,
            report_age_ms: None,
            room_offset_seconds: None,
        }
    }

    /// Defensively applies the same media-evidence eligibility contract used
    /// to sanitize reports before server retention.
    pub fn redact_ineligible_media_evidence(&mut self) {
        if self.correlation == Some(ParticipantStatusCorrelation::Superseded) {
            // Superseded rows may retain report freshness, player connection,
            // and coarse phase, but no field derived from the retired playback
            // epoch. This is deliberately stronger than the legacy
            // Uncorrelated case, which may still carry a coarse timestamp.
            self.playback_scope = None;
            self.timeline_kind = None;
            self.position_seconds = None;
            self.logical_paused = None;
            self.playback_rate = None;
            self.paused_for_cache = None;
            self.cache_percent = None;
            self.buffered_ahead_seconds = None;
            self.sample_age_ms = None;
            self.position_sample_age_ms = None;
            self.room_offset_seconds = None;
            return;
        }
        if self.correlation != Some(ParticipantStatusCorrelation::Exact) {
            // A precise ahead/behind value is server-authored only when the
            // report is explicitly correlated to the current authoritative
            // scope. Missing/legacy correlation may keep a timestamp, never
            // an offset.
            self.room_offset_seconds = None;
        }
        if self.timeline_kind != Some(ParticipantTimelineKind::Vod) {
            // A server-derived ahead/behind value is meaningful only on a
            // stable ordinary VOD timeline. Live/unknown timelines may retain
            // their local timestamp, but never room-relative precision.
            self.room_offset_seconds = None;
        }
        let position_eligible =
            self.player_connection
                .zip(self.phase)
                .is_some_and(|(connection, phase)| {
                    participant_status_position_evidence_is_eligible(connection, phase)
                });
        let buffer_eligible =
            self.player_connection
                .zip(self.phase)
                .is_some_and(|(connection, phase)| {
                    participant_status_buffer_evidence_is_eligible(connection, phase)
                });
        if !position_eligible {
            self.timeline_kind = None;
            self.position_seconds = None;
            self.logical_paused = None;
            self.playback_rate = None;
            self.position_sample_age_ms = None;
            self.room_offset_seconds = None;
        }
        if !buffer_eligible {
            self.paused_for_cache = None;
            self.cache_percent = None;
            self.buffered_ahead_seconds = None;
        }
        if !position_eligible && !buffer_eligible {
            self.sample_age_ms = None;
        }
    }
}

/// Completeness level used to keep complete snapshot semantics within the
/// smallest advertised client framing limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ParticipantStatusSnapshotMode {
    #[default]
    Full,
    Compact,
    Unavailable,
}

/// Complete current-room snapshot sent on the coalescible periodic State path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub struct ParticipantStatusSnapshot {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "is_full_snapshot_mode")]
    pub mode: ParticipantStatusSnapshotMode,
    pub participants: BTreeMap<String, ParticipantStatusView>,
}

fn is_full_snapshot_mode(mode: &ParticipantStatusSnapshotMode) -> bool {
    *mode == ParticipantStatusSnapshotMode::Full
}

impl ParticipantStatusSnapshot {
    pub fn new(revision: u64, participants: BTreeMap<String, ParticipantStatusView>) -> Self {
        Self {
            revision,
            mode: ParticipantStatusSnapshotMode::Full,
            participants,
        }
    }

    pub fn with_mode(mut self, mode: ParticipantStatusSnapshotMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Bidirectional State extension. Clients send only `report`; servers send
/// only `snapshot`. The opposite-direction field is ignored by each receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub struct ParticipantStatusStateExtension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ParticipantStatusReport>,
    /// Current server-authored room scope. Clients echo it only after their
    /// player has applied the corresponding canonical State.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ParticipantPlaybackScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ParticipantStatusSnapshot>,
}

impl ParticipantStatusStateExtension {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_report(mut self, report: ParticipantStatusReport) -> Self {
        self.report = Some(report);
        self
    }

    pub fn with_scope(mut self, scope: ParticipantPlaybackScope) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn with_snapshot(mut self, snapshot: ParticipantStatusSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }
}

pub(crate) fn insert_extension(
    extra: &mut BTreeMap<String, Value>,
    extension: &ParticipantStatusStateExtension,
) {
    let value = serde_json::to_value(extension)
        .expect("participant status V1 extension types must serialize to JSON");
    extra.insert(SOROTTE_PARTICIPANT_STATUS_V1.to_owned(), value);
}

pub(crate) fn decode_extension(
    extra: &BTreeMap<String, Value>,
) -> serde_json::Result<Option<ParticipantStatusStateExtension>> {
    extra
        .get(SOROTTE_PARTICIPANT_STATUS_V1)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParticipantStatusReportStateExtension {
    #[serde(default)]
    report: Option<ParticipantStatusReport>,
}

/// Decodes only the client-to-server half of the bidirectional extension.
/// Malformed server-only scope/snapshot fields are deliberately ignored.
pub(crate) fn decode_report(
    extra: &BTreeMap<String, Value>,
) -> serde_json::Result<Option<ParticipantStatusReport>> {
    extra
        .get(SOROTTE_PARTICIPANT_STATUS_V1)
        .cloned()
        .map(serde_json::from_value::<ParticipantStatusReportStateExtension>)
        .transpose()
        .map(|extension| extension.and_then(|extension| extension.report))
}
