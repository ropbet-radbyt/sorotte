use super::*;
use crate::ClientParticipantStatusFreshness;
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlayerConnection, ParticipantStatusAvailability,
    ParticipantStatusCorrelation, ParticipantStatusSnapshotMode, ParticipantStatusView,
};

fn status_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .expect("participant-status Hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .expect("capable room member should apply");
    session
}

fn bob_status_state(revision: u64, report_age_ms: u64, phase: &str) -> String {
    format!(
        r#"{{"State":{{"sorotteParticipantStatusV1":{{"snapshot":{{"revision":{revision},"participants":{{"bob":{{"availability":"fresh","playbackScope":{{"mediaGeneration":7,"stateRevision":19}},"playerConnection":"connected","phase":"{phase}","timelineKind":"vod","positionSeconds":42.5,"logicalPaused":false,"playbackRate":1.0,"pausedForCache":false,"cachePercent":35.0,"bufferedAheadSeconds":8.0,"sampleAgeMs":100,"positionSampleAgeMs":100,"reportAgeMs":{report_age_ms},"roomOffsetSeconds":-0.25}}}}}}}}}}}}"#
    )
}

#[test]
fn participant_status_capability_advertising_and_peer_support_are_explicit() {
    let mut features = serde_json::Map::from_iter([(
        "unrelatedFeature".to_owned(),
        serde_json::Value::Bool(true),
    )]);
    ClientSession::advertise_participant_status_v1(&mut features);
    assert_eq!(
        features.get("sorotteParticipantStatusV1"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        features.get("unrelatedFeature"),
        Some(&serde_json::Value::Bool(true))
    );

    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":false}}}}}"#,
        )
        .unwrap();
    assert_eq!(
        session.user_participant_status_v1_supported("bob"),
        Some(true)
    );
    assert_eq!(
        session.user_participant_status_v1_supported("carol"),
        Some(false)
    );
    assert_eq!(
        session.user_participant_status_v1_supported("unknown"),
        None
    );
}

#[test]
fn participant_status_freshness_rejects_negative_and_non_finite_ages() {
    for age in [-1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert_eq!(
            ClientParticipantStatusFreshness::from_report_age_seconds(Some(age)),
            ClientParticipantStatusFreshness::Unknown,
            "invalid age {age:?} must not establish fresh evidence"
        );
    }
}

#[test]
fn participant_status_server_availability_is_a_floor_and_controls_precision() {
    let mut session = status_session();
    for (revision, availability, expected_freshness, expected_phase) in [
        (
            1,
            "fresh",
            ClientParticipantStatusFreshness::Fresh,
            Some(ParticipantPlaybackPhase::Playing),
        ),
        (
            2,
            "delayed",
            ClientParticipantStatusFreshness::Delayed,
            Some(ParticipantPlaybackPhase::Playing),
        ),
        (3, "stale", ClientParticipantStatusFreshness::Stale, None),
    ] {
        session
            .apply_message_json_at(
                &serde_json::json!({
                    "State": {
                        "sorotteParticipantStatusV1": {
                            "snapshot": {
                                "revision": revision,
                                "participants": {
                                    "bob": {
                                        "availability": availability,
                                        "correlation": "exact",
                                        "playerConnection": "connected",
                                        "phase": "playing",
                                        "positionSeconds": 42.5,
                                        "sampleAgeMs": 0,
                                        "positionSampleAgeMs": 0,
                                        "reportAgeMs": 0,
                                        "roomOffsetSeconds": -0.25,
                                    },
                                },
                            },
                        },
                    },
                })
                .to_string(),
                revision as f64,
            )
            .unwrap();
        let status = session
            .user_participant_status_at("bob", revision as f64)
            .unwrap();
        assert_eq!(status.freshness, expected_freshness);
        assert_eq!(status.status.phase, expected_phase);
        assert_eq!(
            status.status.room_offset_seconds,
            (availability == "fresh").then_some(-0.25)
        );
    }
}

#[test]
fn participant_status_precise_offsets_require_explicit_exact_correlation() {
    for correlation in [None, Some(ParticipantStatusCorrelation::Uncorrelated)] {
        let mut wire = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
        wire.correlation = correlation;
        wire.player_connection = Some(ParticipantPlayerConnection::Connected);
        wire.phase = Some(ParticipantPlaybackPhase::Playing);
        wire.position_seconds = Some(42.5);
        wire.report_age_ms = Some(0);
        wire.sample_age_ms = Some(0);
        wire.position_sample_age_ms = Some(0);
        wire.room_offset_seconds = Some(-0.25);

        let projected = crate::ClientParticipantStatusView::from_wire(wire);
        assert_eq!(
            projected.status.room_offset_seconds, None,
            "non-exact correlation {correlation:?} cannot support a precise room offset"
        );
        assert_eq!(
            projected.status.position_seconds,
            Some(42.5),
            "legacy correlation may retain its coarse timestamp"
        );
    }

    let mut superseded = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
    superseded.correlation = Some(ParticipantStatusCorrelation::Superseded);
    superseded.player_connection = Some(ParticipantPlayerConnection::Connected);
    superseded.phase = Some(ParticipantPlaybackPhase::Playing);
    superseded.position_seconds = Some(42.5);
    superseded.report_age_ms = Some(0);
    superseded.sample_age_ms = Some(0);
    superseded.position_sample_age_ms = Some(0);
    superseded.room_offset_seconds = Some(-0.25);
    let superseded = crate::ClientParticipantStatusView::from_wire(superseded);
    assert_eq!(superseded.status.position_seconds, None);
    assert_eq!(superseded.status.room_offset_seconds, None);

    let mut exact = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
    exact.correlation = Some(ParticipantStatusCorrelation::Exact);
    exact.player_connection = Some(ParticipantPlayerConnection::Connected);
    exact.phase = Some(ParticipantPlaybackPhase::Playing);
    exact.position_seconds = Some(42.5);
    exact.report_age_ms = Some(0);
    exact.sample_age_ms = Some(0);
    exact.position_sample_age_ms = Some(0);
    exact.room_offset_seconds = Some(-0.25);
    assert_eq!(
        crate::ClientParticipantStatusView::from_wire(exact)
            .status
            .room_offset_seconds,
        Some(-0.25)
    );
}

#[test]
fn participant_status_direct_aging_fails_closed_for_invalid_elapsed_time() {
    let wire: ParticipantStatusView = serde_json::from_value(serde_json::json!({
        "availability": "fresh",
        "correlation": "exact",
        "playerConnection": "connected",
        "phase": "playing",
        "positionSeconds": 42.5,
        "sampleAgeMs": 0,
        "reportAgeMs": 0,
    }))
    .unwrap();
    let fresh = crate::ClientParticipantStatusView::from_wire(wire);
    assert_eq!(fresh.freshness, ClientParticipantStatusFreshness::Fresh);

    for elapsed in [-1.0, f64::NAN] {
        let retired = fresh.clone().aged_by(elapsed);
        assert_eq!(retired.freshness, ClientParticipantStatusFreshness::Stale);
        assert_eq!(retired.status.position_seconds, None);
    }
}

#[test]
fn participant_status_direct_aging_adds_elapsed_time_to_each_evidence_clock() {
    let sample_boundary: ParticipantStatusView = serde_json::from_value(serde_json::json!({
        "availability": "fresh",
        "correlation": "exact",
        "playerConnection": "connected",
        "phase": "playing",
        "positionSeconds": 42.5,
        "sampleAgeMs": 2900,
        "positionSampleAgeMs": 0,
        "reportAgeMs": 0,
        "roomOffsetSeconds": 0.2,
    }))
    .unwrap();
    let sample_boundary = crate::ClientParticipantStatusView::from_wire(sample_boundary);
    assert_eq!(sample_boundary.status.room_offset_seconds, Some(0.2));
    let aged_sample = sample_boundary.aged_by(0.2);
    assert_eq!(
        aged_sample.status.room_offset_seconds, None,
        "elapsed time must push the general evidence clock past the fresh boundary"
    );
    assert_eq!(aged_sample.status.position_seconds, Some(42.5));

    let position_boundary: ParticipantStatusView = serde_json::from_value(serde_json::json!({
        "availability": "fresh",
        "playerConnection": "connected",
        "phase": "playing",
        "positionSeconds": 42.5,
        "sampleAgeMs": 0,
        "positionSampleAgeMs": 9900,
        "reportAgeMs": 0,
    }))
    .unwrap();
    let position_boundary = crate::ClientParticipantStatusView::from_wire(position_boundary);
    assert_eq!(position_boundary.status.position_seconds, Some(42.5));
    let aged_position = position_boundary.aged_by(0.2);
    assert_eq!(
        aged_position.status.position_seconds, None,
        "elapsed time must push the position-specific clock past the delayed boundary"
    );
    assert_eq!(aged_position.status.position_sample_age_ms, None);
}

#[test]
fn participant_status_projection_advances_exposed_player_evidence_ages() {
    let wire: ParticipantStatusView = serde_json::from_value(serde_json::json!({
        "availability": "fresh",
        "playerConnection": "connected",
        "phase": "playing",
        "positionSeconds": 42.5,
        "sampleAgeMs": 100,
        "positionSampleAgeMs": 200,
        "reportAgeMs": 0,
    }))
    .unwrap();

    let aged = crate::ClientParticipantStatusView::from_wire(wire).aged_by(2.0);
    assert_eq!(aged.status.sample_age_ms, Some(2_100));
    assert_eq!(aged.status.position_sample_age_ms, Some(2_200));
}

#[test]
fn complete_snapshot_projects_ages_and_locally_redacts_stale_observations() {
    let mut session = status_session();
    session
        .apply_message_json_at(&bob_status_state(7, 1_500, "playing"), 100.0)
        .expect("participant status snapshot should apply");

    let delayed = session
        .user_participant_status_at("bob", 104.0)
        .expect("member status should project");
    assert_eq!(
        delayed.status.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(
        delayed.status.phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(delayed.status.position_seconds, Some(42.5));
    assert_eq!(
        delayed.status.room_offset_seconds, None,
        "delayed observations must not retain a precision-looking room offset"
    );
    assert_eq!(delayed.report_age_seconds, Some(5.5));
    assert_eq!(delayed.freshness, ClientParticipantStatusFreshness::Delayed);
    assert_eq!(
        delayed.status.availability,
        ParticipantStatusAvailability::Delayed
    );

    let stale = session
        .user_participant_status_at("bob", 110.0)
        .expect("membership should remain while its observation becomes stale");
    assert_eq!(stale.freshness, ClientParticipantStatusFreshness::Stale);
    assert_eq!(
        stale.status.availability,
        ParticipantStatusAvailability::Stale
    );
    assert_eq!(
        stale.status.player_connection,
        Some(ParticipantPlayerConnection::Connected),
        "stale Sorotte telemetry must not be mislabeled as a player disconnect"
    );
    assert_eq!(stale.status.position_seconds, None);
    assert_eq!(stale.status.room_offset_seconds, None);
    assert_eq!(stale.status.buffered_ahead_seconds, None);

    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"isReady":true,"file":{"name":"movie.mkv"}}}}}"#,
        )
        .expect("membership file update should apply");
    assert!(
        session.user_participant_status_at("bob", 105.0).is_none(),
        "a changed peer file must clear status sampled for the prior media"
    );

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":8,"participants":{}}}}}"#,
            106.0,
        )
        .expect("empty complete snapshot should apply");
    assert!(session.user_participant_status_at("bob", 106.0).is_none());
}

#[test]
fn server_stale_availability_cannot_be_downgraded_by_missing_or_rolled_back_age() {
    let mut session = status_session();
    for (revision, report_age_ms) in [(1, None), (2, Some(0))] {
        let mut view = serde_json::json!({
            "availability": "stale",
            "correlation": "exact",
            "playerConnection": "connected",
            "phase": "playing",
            "positionSeconds": 42.5,
            "bufferedAheadSeconds": 8.0,
        });
        if let Some(report_age_ms) = report_age_ms {
            view.as_object_mut()
                .unwrap()
                .insert("reportAgeMs".to_owned(), serde_json::json!(report_age_ms));
        }
        session
            .apply_message_json_at(
                &serde_json::json!({
                    "State": {
                        "sorotteParticipantStatusV1": {
                            "snapshot": {
                                "revision": revision,
                                "participants": {"bob": view},
                            },
                        },
                    },
                })
                .to_string(),
                10.0 + revision as f64,
            )
            .unwrap();
        let bob = session
            .user_participant_status_at("bob", 10.0 + revision as f64)
            .unwrap();
        assert_eq!(bob.freshness, ClientParticipantStatusFreshness::Stale);
        assert_eq!(
            bob.status.availability,
            ParticipantStatusAvailability::Stale
        );
        assert_eq!(bob.status.position_seconds, None);
        assert_eq!(bob.status.buffered_ahead_seconds, None);
        assert_eq!(
            bob.status.player_connection,
            Some(ParticipantPlayerConnection::Connected)
        );
    }
}

#[test]
fn snapshot_revision_prevents_old_or_duplicate_snapshots_from_resurrecting_status() {
    let mut session = status_session();
    session
        .apply_message_json_at(&bob_status_state(12, 100, "rebuffering"), 10.0)
        .unwrap();
    session
        .apply_message_json_at(&bob_status_state(11, 50, "playing"), 11.0)
        .unwrap();
    let retained = session.user_participant_status_at("bob", 11.0).unwrap();
    assert_eq!(
        retained.status.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":12,"participants":{}}}}}"#,
            12.0,
        )
        .unwrap();
    assert!(session.user_participant_status_at("bob", 12.0).is_some());

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":13,"participants":{}}}}}"#,
            13.0,
        )
        .unwrap();
    assert!(session.user_participant_status_at("bob", 13.0).is_none());

    session
        .apply_message_json_at(&bob_status_state(0, 0, "playing"), 14.0)
        .unwrap();
    assert!(session.user_participant_status_at("bob", 14.0).is_none());
}

#[test]
fn unsupported_and_awaiting_are_explicit_server_owned_states() {
    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":false}}}}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            &serde_json::json!({
                "State": {
                    "sorotteParticipantStatusV1": {
                        "snapshot": {
                            "revision": 1,
                            "participants": {
                                "bob": {"availability": "awaitingReport"},
                                "carol": {"availability": "unsupported"},
                                "mallory": {
                                    "availability": "fresh",
                                    "playerConnection": "connected",
                                    "phase": "playing",
                                    "timelineKind": "vod",
                                    "positionSeconds": 99.0,
                                },
                            },
                        },
                    },
                },
            })
            .to_string(),
            20.0,
        )
        .unwrap();

    assert_eq!(
        session
            .user_participant_status_at("bob", 20.0)
            .unwrap()
            .status
            .availability,
        ParticipantStatusAvailability::AwaitingReport
    );
    assert_eq!(
        session
            .user_participant_status_at("carol", 20.0)
            .unwrap()
            .status
            .availability,
        ParticipantStatusAvailability::Unsupported
    );
    assert!(
        session
            .user_participant_status_at("mallory", 20.0)
            .is_none(),
        "snapshot telemetry must not create membership"
    );
}

#[test]
fn unavailable_snapshot_mode_fails_closed_for_every_capable_room_member() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"mode":"unavailable","participants":{}}}}}"#,
            20.0,
        )
        .unwrap();

    let bob = session
        .user_participant_status_at("bob", 20.0)
        .expect("capable member should receive an explicit unavailable projection");
    assert_eq!(
        bob.status.availability,
        ParticipantStatusAvailability::Unavailable
    );
    assert_eq!(bob.status.player_connection, None);
    assert_eq!(bob.status.phase, None);

    session
        .apply_message_json_at(&bob_status_state(2, 100, "playing"), 21.0)
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 21.0)
            .unwrap()
            .status
            .availability,
        ParticipantStatusAvailability::Fresh,
        "a later full snapshot should recover from the bounded unavailable projection"
    );
}

#[test]
fn views_clear_on_room_switch_capability_loss_and_reconnect() {
    let mut session = status_session();
    session
        .apply_message_json_at(&bob_status_state(1, 0, "rebuffering"), 10.0)
        .unwrap();
    assert!(session.user_participant_status_at("bob", 10.0).is_some());

    session
        .apply_message_json(
            r#"{"Set":{"features":{"username":"bob","features":{"sorotteParticipantStatusV1":false}}}}"#,
        )
        .unwrap();
    assert!(session.user_participant_status_at("bob", 10.0).is_none());

    session
        .apply_message_json_at(&bob_status_state(2, 0, "playing"), 11.0)
        .unwrap();
    session
        .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room switch should apply");
    assert!(session.user_participant_status_at("bob", 11.0).is_none());

    session.reset_sync_state_for_reconnect();
    assert!(!session.server_participant_status_v1_supported());
    assert!(session.user_participant_status_at("bob", 11.0).is_none());
}

#[test]
fn list_position_is_preserved_only_as_a_named_legacy_snapshot() {
    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"List":{"room1":{"alice":{"position":4.0},"bob":{"position":12.5},"invalid":{"position":-1.0}}}}"#,
        )
        .expect("List snapshot should apply");

    assert_eq!(
        session.user_legacy_list_position_snapshot_seconds("bob"),
        Some(12.5)
    );
    assert_eq!(
        session.user_legacy_list_position_snapshot_seconds("invalid"),
        None
    );
    assert!(
        session.user_participant_status_at("bob", 0.0).is_none(),
        "List position must never masquerade as live participant telemetry"
    );
}

#[test]
fn inbound_status_requires_the_active_negotiated_connection() {
    let mut reconnecting = status_session();
    reconnecting
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 10.0)
        .unwrap();
    reconnecting.mark_reconnecting(1);
    reconnecting
        .apply_message_json_at(&bob_status_state(2, 0, "rebuffering"), 11.0)
        .expect("a delayed additive State remains protocol-compatible");
    assert!(
        reconnecting
            .user_participant_status_at("bob", 11.0)
            .is_none(),
        "a retired transport must not repopulate status after reconnect fencing"
    );

    let mut unsupported = ClientSession::default();
    unsupported
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":false}}}"#,
        )
        .unwrap();
    unsupported
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .unwrap();
    unsupported
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 12.0)
        .unwrap();
    assert!(
        unsupported
            .user_participant_status_at("bob", 12.0)
            .is_none(),
        "an extension the active Hello did not negotiate must be ignored"
    );
}

#[test]
fn scope_and_snapshot_apply_transactionally_without_precise_scope_rollback() {
    let mut session = status_session();
    let scoped_snapshot = |revision: u64,
                           transport_revision: u64,
                           phase: &str,
                           mode: ParticipantStatusSnapshotMode| {
        let mut view = serde_json::json!({
            "availability": "fresh",
            "correlation": "exact",
            "playerConnection": "connected",
            "phase": phase,
            "sampleAgeMs": 0,
            "reportAgeMs": 0,
        });
        if mode == ParticipantStatusSnapshotMode::Full {
            view.as_object_mut().unwrap().extend([
                (
                    "playbackScope".to_owned(),
                    serde_json::json!({
                        "mediaGeneration": 7,
                        "stateRevision": transport_revision,
                        "transportRevision": transport_revision,
                    }),
                ),
                ("timelineKind".to_owned(), serde_json::json!("vod")),
                ("positionSeconds".to_owned(), serde_json::json!(42.5)),
                ("positionSampleAgeMs".to_owned(), serde_json::json!(0)),
                ("roomOffsetSeconds".to_owned(), serde_json::json!(-0.25)),
            ]);
        }
        serde_json::json!({
            "State": {
                "sorotteParticipantStatusV1": {
                    "scope": {
                        "mediaGeneration": 7,
                        "stateRevision": transport_revision,
                        "transportRevision": transport_revision,
                    },
                    "snapshot": {
                        "revision": revision,
                        "mode": match mode {
                            ParticipantStatusSnapshotMode::Full => "full",
                            ParticipantStatusSnapshotMode::Compact => "compact",
                            ParticipantStatusSnapshotMode::Unavailable => "unavailable",
                            _ => "unknown",
                        },
                        "participants": {"bob": view},
                    },
                },
            },
        })
        .to_string()
    };

    session
        .apply_message_json_at(
            &scoped_snapshot(10, 1, "playing", ParticipantStatusSnapshotMode::Full),
            10.0,
        )
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 10.0)
            .unwrap()
            .status
            .position_seconds,
        Some(42.5)
    );

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":2,"transportRevision":2}}}}"#,
            10.1,
        )
        .unwrap();
    let superseded = session.user_participant_status_at("bob", 10.1).unwrap();
    assert_eq!(
        superseded.status.correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(superseded.status.position_seconds, None);
    assert_eq!(superseded.status.room_offset_seconds, None);
    assert_eq!(
        session.model.room.participant_status_snapshot_revision,
        Some(10),
        "a scope-only authority update must preserve the snapshot revision fence"
    );

    session
        .apply_message_json_at(
            &scoped_snapshot(11, 3, "rebuffering", ParticipantStatusSnapshotMode::Compact),
            11.0,
        )
        .unwrap();
    let compact = session.user_participant_status_at("bob", 11.0).unwrap();
    assert_eq!(
        compact.status.correlation,
        Some(ParticipantStatusCorrelation::Exact),
        "a compact snapshot bundled with its new scope must not be redacted as retained old data"
    );
    assert_eq!(
        compact.status.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":1,"stateRevision":4,"transportRevision":4},"snapshot":{"revision":12,"mode":"full","participants":{"bob":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":1,"stateRevision":4,"transportRevision":4},"playerConnection":"connected","phase":"playing","timelineKind":"vod","positionSeconds":55.0,"sampleAgeMs":0,"positionSampleAgeMs":0,"reportAgeMs":0}}}}}}"#,
            11.5,
        )
        .unwrap();
    let reset_generation = session.user_participant_status_at("bob", 11.5).unwrap();
    assert_eq!(reset_generation.status.position_seconds, Some(55.0));
    assert_eq!(
        session
            .participant_status_authoritative_scope()
            .and_then(|scope| scope.transport_revision),
        Some(4),
        "the monotonic transport fence outranks independent media-generation allocators"
    );

    session
        .apply_message_json_at(
            &scoped_snapshot(10, 1, "failed", ParticipantStatusSnapshotMode::Full),
            12.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            &scoped_snapshot(13, 2, "failed", ParticipantStatusSnapshotMode::Full),
            12.1,
        )
        .unwrap();
    let retained = session.user_participant_status_at("bob", 12.1).unwrap();
    assert_eq!(
        retained.status.phase,
        Some(ParticipantPlaybackPhase::Playing),
        "neither an old snapshot revision nor a newer snapshot carrying an older scope may roll authority back"
    );
    assert_eq!(
        session
            .participant_status_authoritative_scope()
            .and_then(|scope| scope.transport_revision),
        Some(4)
    );
}

#[test]
fn participant_status_equal_transport_scope_orders_by_state_revision() {
    let mut session = status_session();
    let scoped_snapshot = |snapshot_revision: u64, state_revision: u64, phase: &str| {
        serde_json::json!({
            "State": {
                "sorotteParticipantStatusV1": {
                    "scope": {
                        "mediaGeneration": 7,
                        "stateRevision": state_revision,
                        "transportRevision": 5,
                    },
                    "snapshot": {
                        "revision": snapshot_revision,
                        "participants": {
                            "bob": {
                                "availability": "fresh",
                                "correlation": "exact",
                                "playbackScope": {
                                    "mediaGeneration": 7,
                                    "stateRevision": state_revision,
                                    "transportRevision": 5,
                                },
                                "playerConnection": "connected",
                                "phase": phase,
                                "positionSeconds": 42.5,
                                "sampleAgeMs": 0,
                                "reportAgeMs": 0,
                            },
                        },
                    },
                },
            },
        })
        .to_string()
    };

    session
        .apply_message_json_at(&scoped_snapshot(1, 10, "playing"), 1.0)
        .unwrap();
    session
        .apply_message_json_at(&scoped_snapshot(2, 9, "failed"), 2.0)
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 2.0)
            .unwrap()
            .status
            .phase,
        Some(ParticipantPlaybackPhase::Playing),
        "a lower state revision under the same transport and media fence is stale"
    );

    session
        .apply_message_json_at(&scoped_snapshot(2, 10, "rebuffering"), 2.0)
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 2.0)
            .unwrap()
            .status
            .phase,
        Some(ParticipantPlaybackPhase::Rebuffering),
        "an equal scope may carry a newer complete snapshot"
    );

    session
        .apply_message_json_at(&scoped_snapshot(3, 11, "failed"), 3.0)
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 3.0)
            .unwrap()
            .status
            .phase,
        Some(ParticipantPlaybackPhase::Failed),
        "a higher state revision under the same transport fence is authoritative"
    );
}

#[test]
fn participant_status_scope_mismatches_are_redacted_at_storage_and_projection_boundaries() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":10,"transportRevision":5},"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":9,"transportRevision":5},"playerConnection":"connected","phase":"playing","positionSeconds":42.5,"sampleAgeMs":0,"reportAgeMs":0}}}}}}"#,
            1.0,
        )
        .unwrap();
    let stored = session.model.room.participant_statuses.get("bob").unwrap();
    assert_eq!(
        stored.status.correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(stored.status.position_seconds, None);

    let authoritative_scope = session
        .participant_status_authoritative_scope()
        .expect("scope should be committed transactionally");
    let stored = session
        .model
        .room
        .participant_statuses
        .get_mut("bob")
        .unwrap();
    stored.status.correlation = Some(ParticipantStatusCorrelation::Exact);
    stored.status.playback_scope = Some(
        sorotte_protocol::ParticipantPlaybackScope::new(authoritative_scope.media_generation)
            .with_state_revision(
                authoritative_scope
                    .state_revision
                    .expect("authoritative test scope has a state revision")
                    .saturating_sub(1),
            )
            .with_transport_revision(
                authoritative_scope
                    .transport_revision
                    .expect("authoritative test scope has a transport revision"),
            ),
    );
    stored.status.position_seconds = Some(42.5);

    let projected = session.user_participant_status_at("bob", 1.0).unwrap();
    assert_eq!(
        projected.status.correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(projected.status.position_seconds, None);
}

#[test]
fn participant_status_unavailable_projection_excludes_incapable_and_other_room_users() {
    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":false}},"dave":{"room":{"name":"room2"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"mode":"unavailable","participants":{}}}}}"#,
            1.0,
        )
        .unwrap();

    assert!(session.user_participant_status_at("bob", 1.0).is_some());
    assert!(session.user_participant_status_at("carol", 1.0).is_none());
    assert!(session.user_participant_status_at("dave", 1.0).is_none());
}

#[test]
fn missing_age_and_local_clock_rollback_fail_closed() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"exact","playerConnection":"connected","phase":"playing","positionSeconds":42.5}}}}}}"#,
            100.0,
        )
        .unwrap();
    let missing_age = session.user_participant_status_at("bob", 100.0).unwrap();
    assert_eq!(
        missing_age.freshness,
        ClientParticipantStatusFreshness::Stale
    );
    assert_eq!(missing_age.status.position_seconds, None);

    session
        .apply_message_json_at(&bob_status_state(2, 0, "playing"), 100.0)
        .unwrap();
    let rolled_back = session.user_participant_status_at("bob", 99.0).unwrap();
    assert_eq!(
        rolled_back.freshness,
        ClientParticipantStatusFreshness::Stale
    );
    assert_eq!(rolled_back.status.position_seconds, None);
    assert_eq!(rolled_back.status.room_offset_seconds, None);
    let caught_up = session.user_participant_status_at("bob", 100.0).unwrap();
    assert_eq!(caught_up.freshness, ClientParticipantStatusFreshness::Stale);
    assert_eq!(
        caught_up.status.position_seconds, None,
        "a clock rollback must retire precise evidence for the lifetime of that snapshot"
    );

    session
        .apply_message_json_at(&bob_status_state(3, 0, "playing"), -f64::MAX)
        .unwrap();
    let overflowed = session.user_participant_status_at("bob", f64::MAX).unwrap();
    assert_eq!(
        overflowed.freshness,
        ClientParticipantStatusFreshness::Stale
    );
    assert_eq!(overflowed.status.position_seconds, None);
    assert_eq!(
        session
            .user_participant_status_at("bob", -f64::MAX)
            .unwrap()
            .freshness,
        ClientParticipantStatusFreshness::Stale,
        "finite subtraction overflow must also retire the snapshot irreversibly"
    );
}

#[test]
fn participant_status_age_overflow_is_irreversible_but_large_finite_sums_are_not() {
    let mut overflow = status_session();
    overflow
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 0.0)
        .unwrap();
    overflow
        .model
        .room
        .participant_statuses
        .get_mut("bob")
        .unwrap()
        .report_age_seconds = Some(f64::MAX);
    assert_eq!(
        overflow
            .user_participant_status_at("bob", f64::MAX)
            .unwrap()
            .freshness,
        ClientParticipantStatusFreshness::Stale
    );
    overflow
        .model
        .room
        .participant_statuses
        .get_mut("bob")
        .unwrap()
        .report_age_seconds = Some(0.0);
    let cannot_resurrect = overflow.user_participant_status_at("bob", 0.0).unwrap();
    assert_eq!(
        cannot_resurrect.freshness,
        ClientParticipantStatusFreshness::Stale
    );
    assert_eq!(cannot_resurrect.status.position_seconds, None);

    let mut finite = status_session();
    finite
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 0.0)
        .unwrap();
    finite
        .model
        .room
        .participant_statuses
        .get_mut("bob")
        .unwrap()
        .report_age_seconds = Some(1.0e200);
    assert_eq!(
        finite
            .user_participant_status_at("bob", 1.0e200)
            .unwrap()
            .freshness,
        ClientParticipantStatusFreshness::Stale
    );
    finite
        .model
        .room
        .participant_statuses
        .get_mut("bob")
        .unwrap()
        .report_age_seconds = Some(0.0);
    let remains_usable = finite.user_participant_status_at("bob", 0.0).unwrap();
    assert_eq!(
        remains_usable.freshness,
        ClientParticipantStatusFreshness::Fresh
    );
    assert_eq!(remains_usable.status.position_seconds, Some(42.5));
}

#[test]
fn local_aging_separates_report_freshness_from_player_evidence_age() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","playerConnection":"connected","phase":"playing","positionSeconds":42.5,"sampleAgeMs":2900,"positionSampleAgeMs":2900,"reportAgeMs":0,"roomOffsetSeconds":0.2}}}}}}"#,
            10.0,
        )
        .unwrap();
    let fresh_report = session.user_participant_status_at("bob", 10.2).unwrap();
    assert_eq!(
        fresh_report.freshness,
        ClientParticipantStatusFreshness::Fresh
    );
    assert_eq!(
        fresh_report.status.phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(fresh_report.status.position_seconds, Some(42.5));
    assert_eq!(fresh_report.status.room_offset_seconds, None);

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":2,"participants":{"bob":{"availability":"fresh","playerConnection":"connected","phase":"playing","positionSeconds":42.5,"sampleAgeMs":9900,"positionSampleAgeMs":9900,"reportAgeMs":0}}}}}}"#,
            20.0,
        )
        .unwrap();
    let stale_evidence = session.user_participant_status_at("bob", 20.2).unwrap();
    assert_eq!(
        stale_evidence.freshness,
        ClientParticipantStatusFreshness::Fresh
    );
    assert_eq!(
        stale_evidence.status.phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(stale_evidence.status.position_seconds, None);
}

#[test]
fn fresh_position_clock_survives_stale_unrelated_evidence() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"exact","playerConnection":"connected","phase":"playing","timelineKind":"vod","positionSeconds":42.5,"logicalPaused":false,"playbackRate":1.0,"pausedForCache":false,"bufferedAheadSeconds":8.0,"sampleAgeMs":11000,"positionSampleAgeMs":0,"reportAgeMs":0,"roomOffsetSeconds":0.2}}}}}}"#,
            10.0,
        )
        .unwrap();

    let projected = session.user_participant_status_at("bob", 10.2).unwrap();
    assert_eq!(projected.freshness, ClientParticipantStatusFreshness::Fresh);
    assert_eq!(projected.status.position_seconds, Some(42.5));
    assert_eq!(projected.status.position_sample_age_ms, Some(200));
    assert_eq!(
        projected.status.timeline_kind,
        Some(sorotte_protocol::ParticipantTimelineKind::Vod)
    );
    assert_eq!(projected.status.logical_paused, None);
    assert_eq!(projected.status.playback_rate, None);
    assert_eq!(projected.status.buffered_ahead_seconds, None);
    assert_eq!(projected.status.sample_age_ms, None);
    assert_eq!(projected.status.room_offset_seconds, None);
}

#[test]
fn superseded_wire_rows_retain_only_coarse_lifecycle_evidence() {
    let mut value = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
    value.correlation = Some(ParticipantStatusCorrelation::Superseded);
    value.playback_scope =
        Some(sorotte_protocol::ParticipantPlaybackScope::new(7).with_state_revision(19));
    value.player_connection = Some(ParticipantPlayerConnection::Connected);
    value.phase = Some(ParticipantPlaybackPhase::Playing);
    value.timeline_kind = Some(sorotte_protocol::ParticipantTimelineKind::Vod);
    value.position_seconds = Some(42.5);
    value.logical_paused = Some(false);
    value.playback_rate = Some(1.0);
    value.paused_for_cache = Some(false);
    value.cache_percent = Some(35.0);
    value.buffered_ahead_seconds = Some(8.0);
    value.sample_age_ms = Some(100);
    value.position_sample_age_ms = Some(100);
    value.report_age_ms = Some(0);
    value.room_offset_seconds = Some(-0.25);

    let view = crate::ClientParticipantStatusView::from_wire(value);
    assert_eq!(
        view.status.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(view.status.phase, Some(ParticipantPlaybackPhase::Playing));
    assert_eq!(view.status.playback_scope, None);
    assert_eq!(view.status.position_seconds, None);
    assert_eq!(view.status.logical_paused, None);
    assert_eq!(view.status.playback_rate, None);
    assert_eq!(view.status.buffered_ahead_seconds, None);
    assert_eq!(view.status.sample_age_ms, None);
    assert_eq!(view.status.position_sample_age_ms, None);
    assert_eq!(view.status.room_offset_seconds, None);
}

#[test]
fn lifecycle_reports_without_player_samples_age_from_report_heartbeat_only() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","playerConnection":"disconnected","phase":"failed","reportAgeMs":0}}}}}}"#,
            10.0,
        )
        .unwrap();

    let fresh = session.user_participant_status_at("bob", 10.2).unwrap();
    assert_eq!(fresh.freshness, ClientParticipantStatusFreshness::Fresh);
    assert_eq!(
        fresh.status.player_connection,
        Some(ParticipantPlayerConnection::Disconnected)
    );
    assert_eq!(fresh.status.phase, Some(ParticipantPlaybackPhase::Failed));
    assert_eq!(fresh.status.sample_age_ms, None);

    let stale = session.user_participant_status_at("bob", 20.1).unwrap();
    assert_eq!(stale.freshness, ClientParticipantStatusFreshness::Stale);
    assert_eq!(stale.status.phase, None);
}

#[test]
fn malformed_snapshot_still_applies_valid_advancing_scope_as_an_invalidation() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"playerConnection":"connected","phase":"playing","positionSeconds":42.5,"sampleAgeMs":0,"positionSampleAgeMs":0,"reportAgeMs":0}}}}}}"#,
            1.0,
        )
        .unwrap();
    assert_eq!(
        session
            .user_participant_status_at("bob", 1.0)
            .unwrap()
            .status
            .position_seconds,
        Some(42.5)
    );

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":2,"transportRevision":2},"snapshot":{"revision":2,"participants":{"bob":{"availability":"fresh","phase":"futurePhase"}}}}}}"#,
            2.0,
        )
        .expect("malformed additive snapshot must not reject its valid containing State");

    assert_eq!(
        session
            .participant_status_authoritative_scope()
            .and_then(|scope| scope.transport_revision),
        Some(2)
    );
    let invalidated = session.user_participant_status_at("bob", 2.0).unwrap();
    assert_eq!(
        invalidated.status.correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(invalidated.status.position_seconds, None);
    assert_eq!(
        session.model.room.participant_status_snapshot_revision,
        Some(1)
    );
    assert_eq!(session.drain_compatibility_fallbacks().len(), 1);
}

#[test]
fn malformed_scope_cannot_advance_snapshot_under_previous_epoch() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"playerConnection":"connected","phase":"playing","positionSeconds":42.5,"sampleAgeMs":0,"positionSampleAgeMs":0,"reportAgeMs":0}}}}}}"#,
            1.0,
        )
        .unwrap();

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":"malformed","stateRevision":2,"transportRevision":2},"snapshot":{"revision":2,"participants":{"bob":{"availability":"fresh","correlation":"exact","playerConnection":"connected","phase":"failed","reportAgeMs":0}}}}}}"#,
            2.0,
        )
        .expect("a malformed additive scope must not reject its containing State");

    assert_eq!(
        session.model.room.participant_status_snapshot_revision,
        Some(1),
        "the malformed scope transaction must not advance the snapshot fence"
    );
    assert_eq!(session.participant_status_authoritative_scope(), None);
    assert!(
        session.user_participant_status_at("bob", 2.0).is_none(),
        "old exact evidence must retire when the bundled authority is malformed"
    );
    assert_eq!(session.drain_compatibility_fallbacks().len(), 1);
}

#[test]
fn media_change_preserves_snapshot_revision_tombstone() {
    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .unwrap();
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .unwrap();
    session
        .apply_message_json_at(&bob_status_state(5, 0, "playing"), 5.0)
        .unwrap();
    assert!(session.user_participant_status_at("bob", 5.0).is_some());

    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .unwrap();
    assert!(session.user_participant_status_at("bob", 6.0).is_none());
    assert_eq!(
        session.model.room.participant_status_snapshot_revision,
        Some(5),
        "media invalidation must retain the per-room revision tombstone"
    );

    session
        .apply_message_json_at(&bob_status_state(4, 0, "failed"), 7.0)
        .unwrap();
    assert!(
        session.user_participant_status_at("bob", 7.0).is_none(),
        "an older snapshot must not repopulate evidence after a media change"
    );
    assert_eq!(
        session.model.room.participant_status_snapshot_revision,
        Some(5)
    );
}

#[test]
fn explicit_capability_withdrawal_and_snapshot_modes_fail_closed() {
    let mut session = status_session();
    session
        .apply_message_json(
            r#"{"Set":{"features":{"username":"bob","features":{"sorotteParticipantStatusV1":false}}}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 1.0)
        .unwrap();
    assert!(session.user_participant_status_at("bob", 1.0).is_none());

    session
        .apply_message_json(
            r#"{"Set":{"features":{"username":"bob","features":{"sorotteParticipantStatusV1":true}}}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":2,"mode":"compact","participants":{"bob":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"playerConnection":"connected","phase":"playing","positionSeconds":42.5,"logicalPaused":false,"playbackRate":1.0,"sampleAgeMs":0,"positionSampleAgeMs":0,"reportAgeMs":0,"roomOffsetSeconds":0.2}}}}}}"#,
            2.0,
        )
        .unwrap();
    let compact = session.user_participant_status_at("bob", 2.0).unwrap();
    assert_eq!(
        compact.status.phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(compact.status.playback_scope, None);
    assert_eq!(compact.status.position_seconds, None);
    assert_eq!(compact.status.logical_paused, None);
    assert_eq!(compact.status.sample_age_ms, None);
    assert_eq!(compact.status.room_offset_seconds, None);

    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":3,"mode":"unavailable","participants":{"bob":{"availability":"fresh","playerConnection":"connected","phase":"playing","positionSeconds":99.0,"sampleAgeMs":0,"positionSampleAgeMs":0,"reportAgeMs":0}}}}}}"#,
            3.0,
        )
        .unwrap();
    let unavailable = session.user_participant_status_at("bob", 3.0).unwrap();
    assert_eq!(
        unavailable.status.availability,
        ParticipantStatusAvailability::Unavailable
    );
    assert_eq!(unavailable.status.player_connection, None);
    assert_eq!(unavailable.status.position_seconds, None);
}

#[test]
fn zero_scope_components_reject_the_complete_extension_transaction() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":1,"transportRevision":1},"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","playerConnection":"connected","phase":"playing","reportAgeMs":0}}}}}}"#,
            1.0,
        )
        .unwrap();

    for (revision, scope) in [
        (
            2,
            serde_json::json!({"mediaGeneration": 0, "stateRevision": 2, "transportRevision": 2}),
        ),
        (
            3,
            serde_json::json!({"mediaGeneration": 7, "stateRevision": 0, "transportRevision": 3}),
        ),
        (
            4,
            serde_json::json!({"mediaGeneration": 7, "stateRevision": 4, "transportRevision": 0}),
        ),
    ] {
        session
            .apply_message_json_at(
                &serde_json::json!({
                    "State": {
                        "sorotteParticipantStatusV1": {
                            "scope": scope,
                            "snapshot": {
                                "revision": revision,
                                "participants": {},
                            },
                        },
                    },
                })
                .to_string(),
                revision as f64,
            )
            .unwrap();
        assert_eq!(
            session
                .participant_status_authoritative_scope()
                .and_then(|scope| scope.transport_revision),
            Some(1)
        );
        assert_eq!(
            session.model.room.participant_status_snapshot_revision,
            Some(1)
        );
    }
}

#[test]
fn participant_status_remains_advisory_to_ordinary_playstate() {
    let mut session = status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":12.0,"paused":true,"setBy":"bob"}}}"#,
            1.0,
        )
        .unwrap();
    let before = session.current_room_playstate().cloned();
    session
        .apply_message_json_at(&bob_status_state(1, 0, "playing"), 2.0)
        .unwrap();
    assert_eq!(
        session.current_room_playstate().cloned(),
        before,
        "status-only State must not become playback authority"
    );

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":18.0,"paused":false,"setBy":"bob"},"sorotteParticipantStatusV1":{"snapshot":{"revision":2,"participants":{}}}}}"#,
            3.0,
        )
        .unwrap();
    let bundled = session.current_room_playstate().unwrap();
    assert_eq!(bundled.position, Some(18.0));
    assert_eq!(bundled.paused, Some(false));
}

#[test]
fn malformed_participant_status_fallback_never_retains_attacker_tokens() {
    let mut session = status_session();
    assert!(session.drain_compatibility_fallbacks().is_empty());

    let canary = "attacker-controlled-participant-phase-canary";
    let message = serde_json::json!({
        "State": {
            "sorotteParticipantStatusV1": {
                "snapshot": {
                    "revision": 1,
                    "participants": {
                        "bob": {
                            "availability": "fresh",
                            "phase": canary,
                        },
                    },
                },
            },
        },
    })
    .to_string();

    session
        .apply_message_json(&message)
        .expect("malformed additive status must not reject the containing State");
    let fallbacks = session.drain_compatibility_fallbacks();
    assert_eq!(fallbacks.len(), 1);
    let rendered = format!("{fallbacks:?}");
    assert!(rendered.contains("IgnoredInvalidFeatures"));
    assert!(rendered.contains("State.sorotteParticipantStatusV1"));
    assert!(!rendered.contains(canary));
}
