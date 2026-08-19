use super::*;

pub const PARTICIPANT_STATUS_FRESH_SECONDS: f64 = 3.0;
pub const PARTICIPANT_STATUS_DELAYED_SECONDS: f64 = 10.0;

/// Client-side freshness classification for advisory participant status.
///
/// Downstream matches must retain a fallback for future additive states.
///
/// ```compile_fail
/// use sorotte_client_core::ClientParticipantStatusFreshness;
///
/// fn label(value: ClientParticipantStatusFreshness) -> &'static str {
///     match value {
///         ClientParticipantStatusFreshness::Unknown => "unknown",
///         ClientParticipantStatusFreshness::Fresh => "fresh",
///         ClientParticipantStatusFreshness::Delayed => "delayed",
///         ClientParticipantStatusFreshness::Stale => "stale",
///     }
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ClientParticipantStatusFreshness {
    #[default]
    Unknown,
    Fresh,
    Delayed,
    Stale,
}

impl ClientParticipantStatusFreshness {
    pub fn from_report_age_seconds(report_age_seconds: Option<f64>) -> Self {
        match report_age_seconds.filter(|age| age.is_finite() && *age >= 0.0) {
            None => Self::Unknown,
            Some(age) if age <= PARTICIPANT_STATUS_FRESH_SECONDS => Self::Fresh,
            Some(age) if age <= PARTICIPANT_STATUS_DELAYED_SECONDS => Self::Delayed,
            Some(_) => Self::Stale,
        }
    }
}

/// Sanitized client projection of one server-authored participant-status row.
///
/// Construct values through [`Self::from_wire`] so additive fields can be
/// normalized without requiring downstream struct literals.
///
/// ```compile_fail
/// use sorotte_client_core::{
///     ClientParticipantStatusFreshness, ClientParticipantStatusView,
/// };
/// use sorotte_protocol::{ParticipantStatusAvailability, ParticipantStatusView};
///
/// let view = ClientParticipantStatusView {
///     status: ParticipantStatusView::new(ParticipantStatusAvailability::Unavailable),
///     report_age_seconds: None,
///     freshness: ClientParticipantStatusFreshness::Unknown,
/// };
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ClientParticipantStatusView {
    pub status: ParticipantStatusView,
    pub report_age_seconds: Option<f64>,
    pub freshness: ClientParticipantStatusFreshness,
}

impl ClientParticipantStatusView {
    pub fn from_wire(mut value: ParticipantStatusView) -> Self {
        if !matches!(
            value.availability,
            ParticipantStatusAvailability::Fresh
                | ParticipantStatusAvailability::Delayed
                | ParticipantStatusAvailability::Stale
        ) {
            // Capability and population-state projections are not player
            // observations. Contradictory optional fields must never turn an
            // Unsupported/Awaiting/Unavailable row into trusted telemetry.
            value.correlation = None;
            value.playback_scope = None;
            value.player_connection = None;
            value.phase = None;
            value.timeline_kind = None;
            value.position_seconds = None;
            value.logical_paused = None;
            value.playback_rate = None;
            value.paused_for_cache = None;
            value.cache_percent = None;
            value.buffered_ahead_seconds = None;
            value.sample_age_ms = None;
            value.position_sample_age_ms = None;
            value.report_age_ms = None;
            value.room_offset_seconds = None;
        }
        // The public protocol sanitizer owns the shared correlation contract:
        // legacy uncorrelated rows may retain coarse media evidence but never
        // an offset, while superseded rows lose all retired-epoch precision.
        value.redact_ineligible_media_evidence();
        // Report freshness and player-evidence freshness are independent.
        // Lifecycle-only reports intentionally omit sampleAgeMs, but remain
        // current while the server continues receiving them.
        let report_age_seconds = value.report_age_ms.map(|age| age as f64 / 1_000.0);
        // The server-owned availability is a lower bound on staleness. A
        // missing age cannot establish freshness and therefore fails closed
        // until a complete replacement snapshot arrives.
        let age_freshness = report_age_seconds
            .map_or(ClientParticipantStatusFreshness::Stale, |age| {
                ClientParticipantStatusFreshness::from_report_age_seconds(Some(age))
            });
        let freshness = match value.availability {
            ParticipantStatusAvailability::Fresh => age_freshness,
            ParticipantStatusAvailability::Delayed => {
                ClientParticipantStatusFreshness::Delayed.max(age_freshness)
            }
            ParticipantStatusAvailability::Stale => ClientParticipantStatusFreshness::Stale,
            _ => ClientParticipantStatusFreshness::Unknown,
        };
        let mut view = Self {
            status: value,
            report_age_seconds,
            freshness,
        };
        view.synchronize_age_derived_availability();
        view.synchronize_evidence_age(0.0);
        view
    }

    pub(crate) fn retain_compact_snapshot_fields(&mut self) {
        // Compact snapshots carry only availability, correlation, connection,
        // coarse phase, and the clocks needed to age those values. Treat any
        // extra precision as an invalid sender contradiction, not an optional
        // extension of compact mode.
        self.status.playback_scope = None;
        self.clear_media_evidence();
    }

    fn synchronize_age_derived_availability(&mut self) {
        if !matches!(
            self.status.availability,
            ParticipantStatusAvailability::Fresh
                | ParticipantStatusAvailability::Delayed
                | ParticipantStatusAvailability::Stale
        ) {
            return;
        }
        self.status.availability = match self.freshness {
            ClientParticipantStatusFreshness::Fresh => ParticipantStatusAvailability::Fresh,
            ClientParticipantStatusFreshness::Delayed => ParticipantStatusAvailability::Delayed,
            ClientParticipantStatusFreshness::Stale => ParticipantStatusAvailability::Stale,
            ClientParticipantStatusFreshness::Unknown => self.status.availability,
        };
        if self.freshness != ClientParticipantStatusFreshness::Fresh {
            // Precise offsets are useful only while the report remains in the
            // server's strict fresh window. Local aging must fail closed at
            // the same boundary instead of retaining a formerly fresh value.
            self.status.room_offset_seconds = None;
        }
        if self.freshness == ClientParticipantStatusFreshness::Stale {
            self.status.playback_scope = None;
            self.status.phase = None;
            self.status.timeline_kind = None;
            self.status.position_seconds = None;
            self.status.logical_paused = None;
            self.status.playback_rate = None;
            self.status.paused_for_cache = None;
            self.status.cache_percent = None;
            self.status.buffered_ahead_seconds = None;
            self.status.sample_age_ms = None;
            self.status.position_sample_age_ms = None;
            self.status.room_offset_seconds = None;
        }
    }

    pub(crate) fn aged_by(mut self, elapsed_seconds: f64) -> Self {
        if !elapsed_seconds.is_finite() || elapsed_seconds < 0.0 {
            return self.fail_closed_stale();
        }
        self.report_age_seconds = match self.report_age_seconds {
            Some(age) => {
                let aged = age + elapsed_seconds;
                if !aged.is_finite() {
                    return self.fail_closed_stale();
                }
                Some(aged)
            }
            None => None,
        };
        if matches!(
            self.status.availability,
            ParticipantStatusAvailability::Fresh
                | ParticipantStatusAvailability::Delayed
                | ParticipantStatusAvailability::Stale
        ) {
            self.freshness =
                self.freshness
                    .max(ClientParticipantStatusFreshness::from_report_age_seconds(
                        self.report_age_seconds,
                    ));
            self.synchronize_age_derived_availability();
        }
        self.synchronize_evidence_age(elapsed_seconds);
        self
    }

    fn synchronize_evidence_age(&mut self, elapsed_seconds: f64) {
        // `aged_by` is the sole caller and rejects invalid elapsed time before
        // mutating either report or player-evidence clocks. Rust's float-to-
        // integer conversion saturates an overflowing positive product at
        // `u64::MAX`, which is exactly the fail-closed evidence age.
        let elapsed_millis = (elapsed_seconds * 1_000.0).ceil() as u64;
        self.status.sample_age_ms = self
            .status
            .sample_age_ms
            .map(|age| age.saturating_add(elapsed_millis));
        self.status.position_sample_age_ms = self
            .status
            .position_sample_age_ms
            .map(|age| age.saturating_add(elapsed_millis));

        let evidence_age_seconds = self.status.sample_age_ms.map(|age| age as f64 / 1_000.0);
        let position_age_seconds = self
            .status
            .position_sample_age_ms
            .map(|age| age as f64 / 1_000.0);
        if !evidence_age_seconds.is_some_and(|age| age <= PARTICIPANT_STATUS_FRESH_SECONDS)
            || !position_age_seconds.is_some_and(|age| age <= PARTICIPANT_STATUS_FRESH_SECONDS)
        {
            self.status.room_offset_seconds = None;
        }
        if !evidence_age_seconds.is_some_and(|age| age <= PARTICIPANT_STATUS_DELAYED_SECONDS) {
            // sampleAgeMs describes the oldest retained non-position
            // evidence. A fresh sparse position has its own clock and must not
            // be discarded merely because buffer/rate/cache evidence is old.
            self.clear_non_position_media_evidence();
        }

        if !position_age_seconds.is_some_and(|age| age <= PARTICIPANT_STATUS_DELAYED_SECONDS) {
            self.clear_position_evidence();
        } else if !position_age_seconds.is_some_and(|age| age <= PARTICIPANT_STATUS_FRESH_SECONDS) {
            self.status.room_offset_seconds = None;
        }
    }

    fn clear_non_position_media_evidence(&mut self) {
        self.status.logical_paused = None;
        self.status.playback_rate = None;
        self.status.paused_for_cache = None;
        self.status.cache_percent = None;
        self.status.buffered_ahead_seconds = None;
        self.status.sample_age_ms = None;
        self.status.room_offset_seconds = None;
    }

    fn clear_position_evidence(&mut self) {
        self.status.timeline_kind = None;
        self.status.position_seconds = None;
        self.status.position_sample_age_ms = None;
        self.status.room_offset_seconds = None;
    }

    fn clear_media_evidence(&mut self) {
        self.clear_non_position_media_evidence();
        self.clear_position_evidence();
    }

    pub(crate) fn fail_closed_stale(mut self) -> Self {
        if matches!(
            self.status.availability,
            ParticipantStatusAvailability::Fresh
                | ParticipantStatusAvailability::Delayed
                | ParticipantStatusAvailability::Stale
        ) {
            self.freshness = ClientParticipantStatusFreshness::Stale;
            self.synchronize_age_derived_availability();
        }
        self
    }

    pub(crate) fn redact_precise_scope_evidence(&mut self) {
        if self.status.correlation == Some(ParticipantStatusCorrelation::Exact) {
            self.status.correlation = Some(ParticipantStatusCorrelation::Superseded);
        }
        self.status.playback_scope = None;
        self.clear_media_evidence();
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientUserView {
    pub room: Option<String>,
    pub ready: Option<bool>,
    pub file: Option<SharedFile>,
    pub capabilities: Option<PeerCapabilities>,
    pub controller: bool,
}

#[derive(Clone, PartialEq, Default)]
pub struct ClientMediaMatchPeerFileState {
    pub username: String,
    pub has_file: bool,
    pub file_name: Option<String>,
    pub file_size: Option<FileSize>,
    pub file_duration: Option<f64>,
    pub media_match_signature: Option<MediaMatchWireSignature>,
}

impl std::fmt::Debug for ClientMediaMatchPeerFileState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientMediaMatchPeerFileState")
            .field("username", &self.username)
            .field("has_file", &self.has_file)
            .field(
                "file_name",
                &self
                    .file_name
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("file_size", &self.file_size)
            .field("file_duration", &self.file_duration)
            .field("media_match_signature", &self.media_match_signature)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct RoomPlaylistView {
    pub files: Vec<String>,
    pub index: Option<i64>,
    pub set_by: Option<String>,
    pub revision: u64,
}

impl std::fmt::Debug for RoomPlaylistView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomPlaylistView")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .field("set_by", &self.set_by)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomPlaystateView {
    pub position: Option<f64>,
    pub paused: Option<bool>,
    pub do_seek: Option<bool>,
    pub set_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPlaystateAuthority {
    LegacyRemoteUser,
    LegacyLocalEcho,
    ServerBarrier {
        media_generation: u64,
        state_revision: Option<u64>,
    },
    ServerBufferingPolicy {
        media_generation: u64,
    },
}
