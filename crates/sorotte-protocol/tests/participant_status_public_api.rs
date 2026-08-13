use std::collections::BTreeMap;

use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlaybackScope, ParticipantPlayerConnection,
    ParticipantStatusAvailability, ParticipantStatusCorrelation, ParticipantStatusReport,
    ParticipantStatusSnapshot, ParticipantStatusStateExtension, ParticipantStatusView,
    ParticipantTimelineKind, StatePayload,
};

fn downstream_availability_is_report_derived(value: ParticipantStatusAvailability) -> bool {
    matches!(
        value,
        ParticipantStatusAvailability::Fresh
            | ParticipantStatusAvailability::Delayed
            | ParticipantStatusAvailability::Stale
    )
}

#[test]
fn downstream_view_sanitizer_exposes_offsets_only_for_exact_correlation() {
    for (correlation, expected_offset) in [
        (None, None),
        (Some(ParticipantStatusCorrelation::Uncorrelated), None),
        (Some(ParticipantStatusCorrelation::Superseded), None),
        (Some(ParticipantStatusCorrelation::Exact), Some(0.25)),
    ] {
        let mut view = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
        view.correlation = correlation;
        view.player_connection = Some(ParticipantPlayerConnection::Connected);
        view.phase = Some(ParticipantPlaybackPhase::Playing);
        view.timeline_kind = Some(ParticipantTimelineKind::Vod);
        view.position_seconds = Some(42.0);
        view.logical_paused = Some(false);
        view.playback_rate = Some(1.0);
        view.sample_age_ms = Some(20);
        view.position_sample_age_ms = Some(10);
        view.room_offset_seconds = Some(0.25);

        view.redact_ineligible_media_evidence();

        assert_eq!(view.room_offset_seconds, expected_offset);
        if correlation == Some(ParticipantStatusCorrelation::Superseded) {
            assert_eq!(view.position_seconds, None);
        } else {
            assert_eq!(view.position_seconds, Some(42.0));
        }
    }
}

#[test]
fn downstream_consumer_uses_extensible_participant_status_builders() {
    let scope = ParticipantPlaybackScope::new(7)
        .with_state_revision(19)
        .with_transport_revision(3);
    let report = ParticipantStatusReport::new(
        1,
        ParticipantPlayerConnection::Connected,
        ParticipantPlaybackPhase::Playing,
    )
    .with_playback_scope(scope)
    .with_timeline_kind(ParticipantTimelineKind::Vod)
    .with_position_seconds(42.5)
    .with_playback_rate(1.0)
    .with_sample_age_ms(250)
    .with_position_sample_age_ms(100);
    let mut participants = BTreeMap::new();
    participants.insert(
        "alice".to_owned(),
        ParticipantStatusView::new(ParticipantStatusAvailability::Fresh),
    );
    let snapshot = ParticipantStatusSnapshot::new(1, participants);
    let state = StatePayload::new().with_participant_status_v1(
        ParticipantStatusStateExtension::new()
            .with_scope(scope)
            .with_report(report)
            .with_snapshot(snapshot),
    );

    let decoded = state
        .participant_status_v1()
        .expect("public participant-status payload should decode")
        .expect("public participant-status extension should be present");
    assert_eq!(decoded.scope, Some(scope));
    assert_eq!(
        decoded.report.expect("report").position_sample_age_ms,
        Some(100)
    );
    assert!(downstream_availability_is_report_derived(
        decoded
            .snapshot
            .expect("snapshot")
            .participants
            .get("alice")
            .expect("alice")
            .availability
    ));
}
