use std::collections::BTreeMap;

use sorotte_client_core::{
    ClientParticipantStatusFreshness, ClientParticipantStatusView, ClientUserView, PeerCapabilities,
};
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlaybackScope, ParticipantPlayerConnection,
    ParticipantStatusAvailability, ParticipantStatusReport, ParticipantStatusSnapshot,
    ParticipantStatusStateExtension, ParticipantStatusView,
};

#[test]
fn additive_participant_status_models_are_constructible_without_public_literals() {
    let scope = ParticipantPlaybackScope::new(7)
        .with_state_revision(11)
        .with_transport_revision(13);
    let report = ParticipantStatusReport::new(
        1,
        ParticipantPlayerConnection::Connected,
        ParticipantPlaybackPhase::Playing,
    )
    .with_playback_scope(scope)
    .with_position_seconds(42.0)
    .with_sample_age_ms(20)
    .with_position_sample_age_ms(10);

    let mut view = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
    view.player_connection = Some(ParticipantPlayerConnection::Connected);
    view.phase = Some(ParticipantPlaybackPhase::Playing);
    view.report_age_ms = Some(0);
    let client_view = ClientParticipantStatusView::from_wire(view.clone());
    let snapshot = ParticipantStatusSnapshot::new(1, BTreeMap::from([("alice".to_owned(), view)]));
    let extension = ParticipantStatusStateExtension::new()
        .with_report(report.clone())
        .with_scope(scope)
        .with_snapshot(snapshot);
    assert!(extension.report.is_some());

    let freshness = match client_view.freshness {
        ClientParticipantStatusFreshness::Fresh => "fresh",
        _ => "other-or-future",
    };
    assert_eq!(freshness, "fresh");

    let peer = PeerCapabilities {
        shared_playlists: true,
        chat: true,
        feature_list: true,
        readiness: true,
        managed_rooms: true,
        persistent_rooms: true,
        media_match: true,
        plex_playlist_uris: true,
        remote_readiness: true,
        playback_barrier_v1: true,
        readiness_v2: true,
        ui_mode: Some("gui".to_owned()),
    };
    let user = ClientUserView {
        room: Some("room".to_owned()),
        ready: Some(true),
        file: None,
        capabilities: Some(peer),
        controller: false,
    };
    assert_eq!(user.room.as_deref(), Some("room"));

    let phase_label = match report.phase {
        ParticipantPlaybackPhase::Playing => "playing",
        _ => "other-or-future",
    };
    assert_eq!(phase_label, "playing");
}
