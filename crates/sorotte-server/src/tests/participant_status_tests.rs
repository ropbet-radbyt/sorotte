use std::collections::BTreeMap;

use sorotte_client_core::{ClientParticipantStatusFreshness, ClientSession};
use sorotte_lifecycle_evidence::{Disposition, Trigger};
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlaybackScope, ParticipantPlayerConnection,
    ParticipantStatusAvailability, ParticipantStatusCorrelation, ParticipantStatusReport,
    ParticipantStatusSnapshot, ParticipantStatusSnapshotMode, ParticipantStatusStateExtension,
    ParticipantStatusView, ParticipantTimelineKind, PlaybackBarrierPhase, PlaystatePayload,
    ProtocolMessage, SOROTTE_PARTICIPANT_STATUS_V1, StatePayload, encode_message_line,
};

use super::*;
use crate::{
    PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS, PARTICIPANT_STATUS_MAX_PLAYBACK_RATE,
    PARTICIPANT_STATUS_MAX_POSITION_SECONDS, PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS,
    PROTOCOL_TIMEOUT_SECONDS, RoomPlaybackState, ServerCompatibilityFallback,
    ServerOutboundDelivery, capture_server_lifecycle_transitions,
};

#[test]
fn participant_status_retention_preserves_public_server_send_sync_contracts() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<ServerRuntime>();
    assert_send_sync::<ServerApp>();
}

fn hello(username: &str, room: &str, participant_status: bool, playback_barrier: bool) -> String {
    json!({
        "Hello": {
            "username": username,
            "room": { "name": room },
            "version": "1.2.255",
            "features": {
                SOROTTE_PARTICIPANT_STATUS_V1: participant_status,
                "sorottePlaybackBarrierV1": playback_barrier,
            },
        },
    })
    .to_string()
}

fn status_report(sequence: u64, phase: ParticipantPlaybackPhase) -> ParticipantStatusReport {
    ParticipantStatusReport::new(sequence, ParticipantPlayerConnection::Connected, phase)
        .with_sample_age_ms(0)
}

fn scoped_vod_report(
    sequence: u64,
    media_generation: u64,
    state_revision: u64,
    transport_revision: u64,
    phase: ParticipantPlaybackPhase,
    position_seconds: f64,
) -> ParticipantStatusReport {
    status_report(sequence, phase)
        .with_playback_scope(
            ParticipantPlaybackScope::new(media_generation)
                .with_state_revision(state_revision)
                .with_transport_revision(transport_revision),
        )
        .with_timeline_kind(ParticipantTimelineKind::Vod)
        .with_position_seconds(position_seconds)
        .with_logical_paused(false)
        .with_playback_rate(1.0)
        .with_paused_for_cache(false)
        .with_cache_percent(50.0)
        .with_buffered_ahead_seconds(8.0)
        .with_sample_age_ms(0)
        .with_position_sample_age_ms(0)
}

fn send_status(
    runtime: &mut ServerRuntime,
    client_id: &str,
    extension: ParticipantStatusStateExtension,
) -> Vec<ProtocolMessage> {
    runtime
        .handle_protocol_message(
            client_id,
            ProtocolMessage::state(StatePayload::new().with_participant_status_v1(extension)),
        )
        .expect("participant status State should be handled")
}

fn periodic_snapshot(
    runtime: &mut ServerRuntime,
    client_id: &str,
    now_seconds: f64,
) -> Option<ParticipantStatusSnapshot> {
    let message = runtime.periodic_state_sync_message_for_client_at(
        client_id,
        0.0,
        true,
        None,
        now_seconds,
    )?;
    let ProtocolMessage::State(message) = message else {
        panic!("periodic sync should be State");
    };
    message
        .state
        .participant_status_v1()
        .expect("server participant status snapshot should decode")
        .and_then(|extension| extension.snapshot)
}

fn acknowledge_current_server_counter(runtime: &mut ServerRuntime, client_id: &str) {
    let counter = runtime.server_ignoring_counter(client_id);
    runtime.acknowledge_server_ignoring_counter(client_id, counter);
}

fn start_new_media(runtime: &mut ServerRuntime, client_id: &str, request_nonce: u64) -> u64 {
    let request_id = format!("participant-status-media-{request_nonce}");
    runtime
        .handle_line_fanout(
            client_id,
            &json!({
                "Set": {
                    "sorottePlaybackBarrierV1": {
                        "prepare": {
                            "mediaGeneration": 0,
                            "requestNonce": request_nonce,
                            "requestId": request_id,
                            "loadIntent": "newPlayback",
                            "logicalMediaId": format!("media-{request_nonce}"),
                            "targetPosition": 100.0,
                            "policy": "controller",
                        },
                    },
                },
            })
            .to_string(),
        )
        .expect("new media barrier should start");
    let room = runtime.sessions[client_id].room.clone();
    runtime.room_playback_barriers[&room]
        .prepare
        .media_generation
}

#[test]
fn participant_status_feature_is_advertised_and_client_capability_is_parsed() {
    let mut runtime = ServerRuntime::default();
    let lines = runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .expect("hello should establish status-capable session");

    assert!(runtime.sessions["alice"].capabilities.participant_status_v1);
    let advertised = lines.iter().find_map(|line| {
        let message = decode_message_line(line).ok()?;
        let ProtocolMessage::Hello(message) = message else {
            return None;
        };
        message
            .hello
            .features
            .and_then(|features| features.get(SOROTTE_PARTICIPANT_STATUS_V1).cloned())
    });
    assert_eq!(advertised, Some(Value::Bool(true)));
}

#[test]
fn server_direction_ignores_malformed_snapshot_and_retains_valid_report() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();

    runtime
        .handle_line(
            "alice",
            r#"{"State":{"sorotteParticipantStatusV1":{"report":{"reportSequence":7,"playerConnection":"connected","phase":"playing","timelineKind":"unknown"},"snapshot":{"revision":8,"participants":{"mallory":{"availability":"futureValue"}}}}}}"#,
        )
        .expect("malformed opposite-direction data must not poison a valid report");

    assert_eq!(
        runtime.client_participant_status["alice"]
            .report
            .report_sequence,
        7
    );
}

#[test]
fn two_client_wire_snapshot_is_consumed_as_exact_advisory_client_state() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let bob_hello_lines = runtime
        .handle_line("bob-transport", &hello("bob", "room", true, false))
        .expect("observing participant should negotiate over the wire");

    let mut bob_session = ClientSession::default();
    for line in bob_hello_lines {
        bob_session
            .apply_message_json_at(&line, 100.0)
            .expect("the real server handshake should initialize client membership");
    }
    let alice_join_fanout = runtime
        .handle_line_fanout("alice-transport", &hello("alice", "room", true, false))
        .expect("reporting participant should negotiate over the wire");
    for line in alice_join_fanout
        .into_iter()
        .filter(|line| line.client_id == "bob-transport")
    {
        bob_session
            .apply_message_json_at(&line.line, 100.0)
            .expect("the real join fanout should establish observer membership");
    }
    acknowledge_current_server_counter(&mut runtime, "bob-transport");

    let baseline = runtime
        .periodic_state_sync_message_for_client_at(
            "bob-transport",
            42.0,
            true,
            Some("alice"),
            100.0,
        )
        .expect("the observer should receive a baseline periodic State");
    let ProtocolMessage::State(baseline_state) = &baseline else {
        panic!("periodic synchronization must remain a State message");
    };
    assert_eq!(
        baseline_state
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.transport_revision().ok().flatten()),
        runtime.transport_authority_revision_for_room("room"),
        "periodic playstate must carry the room transport revision that owns its sample"
    );
    bob_session
        .apply_message_json_at(&encode_message_line(&baseline).unwrap(), 100.0)
        .expect("the baseline periodic State should apply");
    let playback_before_status = (
        bob_session.local_position_seconds(),
        bob_session.local_paused(),
    );

    let scope = runtime.room_participant_status_scopes["room"].to_wire(None);
    let report_line = encode_message_line(&ProtocolMessage::state(
        StatePayload::new().with_participant_status_v1(
            ParticipantStatusStateExtension::new().with_report(
                status_report(1, ParticipantPlaybackPhase::Rebuffering)
                    .with_playback_scope(scope)
                    .with_timeline_kind(ParticipantTimelineKind::Vod)
                    .with_paused_for_cache(true)
                    .with_sample_age_ms(0),
            ),
        ),
    ))
    .unwrap();
    runtime
        .handle_line("alice-transport", &report_line)
        .expect("the authenticated report should traverse the server wire decoder");

    let snapshot_state = runtime
        .periodic_state_sync_message_for_client_at(
            "bob-transport",
            42.0,
            true,
            Some("alice"),
            100.1,
        )
        .expect("the observer should receive the server-authored snapshot");
    bob_session
        .apply_message_json_at(&encode_message_line(&snapshot_state).unwrap(), 100.1)
        .expect("the real periodic snapshot should be accepted by ClientSession");

    let alice = bob_session
        .user_participant_status_at("alice", 100.1)
        .expect("the negotiated member should project from the complete snapshot");
    assert_eq!(alice.freshness, ClientParticipantStatusFreshness::Fresh);
    assert_eq!(
        alice.status.correlation,
        Some(ParticipantStatusCorrelation::Exact)
    );
    assert_eq!(
        alice.status.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );
    assert_eq!(
        (
            bob_session.local_position_seconds(),
            bob_session.local_paused(),
        ),
        playback_before_status,
        "participant telemetry must remain advisory to canonical playback"
    );

    let rollback_state = runtime
        .periodic_state_sync_message_for_client_at("bob-transport", 42.0, true, Some("alice"), 99.0)
        .expect("rollback projection should remain a valid periodic State");
    bob_session
        .apply_message_json_at(&encode_message_line(&rollback_state).unwrap(), 100.2)
        .unwrap();
    assert_eq!(
        bob_session
            .user_participant_status_at("alice", 100.2)
            .unwrap()
            .freshness,
        ClientParticipantStatusFreshness::Stale
    );

    let catchup_state = runtime
        .periodic_state_sync_message_for_client_at(
            "bob-transport",
            42.0,
            true,
            Some("alice"),
            100.2,
        )
        .expect("catch-up projection should remain a valid periodic State");
    bob_session
        .apply_message_json_at(&encode_message_line(&catchup_state).unwrap(), 100.3)
        .unwrap();
    let caught_up = bob_session
        .user_participant_status_at("alice", 100.3)
        .unwrap();
    assert_eq!(
        caught_up.freshness,
        ClientParticipantStatusFreshness::Stale,
        "the same retained report must not become fresh after server clock rollback"
    );
    assert_eq!(caught_up.status.phase, None);
}

#[test]
fn real_join_metadata_and_same_name_replacement_start_a_new_client_evidence_epoch() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let mut bob_session = ClientSession::default();
    for line in runtime
        .handle_line("bob-transport", &hello("bob", "room", true, false))
        .unwrap()
    {
        bob_session.apply_message_json_at(&line, 100.0).unwrap();
    }
    let alice_join = runtime
        .handle_line_fanout("alice-transport", &hello("alice", "room", true, false))
        .unwrap();
    for line in alice_join
        .into_iter()
        .filter(|line| line.client_id == "bob-transport")
    {
        bob_session
            .apply_message_json_at(&line.line, 100.0)
            .unwrap();
    }
    assert_eq!(
        bob_session.user_participant_status_v1_supported("alice"),
        Some(true),
        "join-event features must establish peer capability immediately"
    );

    send_status(
        &mut runtime,
        "alice-transport",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    let snapshot = runtime
        .periodic_state_sync_message_for_client_at("bob-transport", 0.0, true, Some("bob"), 100.1)
        .unwrap();
    bob_session
        .apply_message_json_at(&encode_message_line(&snapshot).unwrap(), 100.1)
        .unwrap();
    assert!(
        bob_session
            .user_participant_status_at("alice", 100.1)
            .is_some()
    );

    let replacement = runtime
        .handle_line_fanout("alice-transport", &hello("alice", "room", true, false))
        .expect("same-transport Hello replacement should be accepted");
    for line in replacement
        .into_iter()
        .filter(|line| line.client_id == "bob-transport")
    {
        bob_session
            .apply_message_json_at(&line.line, 100.2)
            .unwrap();
    }
    assert_eq!(
        bob_session.user_participant_status_v1_supported("alice"),
        Some(true)
    );
    assert!(
        bob_session
            .user_participant_status_at("alice", 100.2)
            .is_none(),
        "replacement join must not preserve the retired connection's status"
    );
}

#[test]
fn two_client_wire_lifecycle_reports_remain_fresh_without_player_samples() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let bob_hello_lines = runtime
        .handle_line("bob-transport", &hello("bob", "room", true, false))
        .expect("observing participant should negotiate over the wire");

    let mut bob_session = ClientSession::default();
    for line in bob_hello_lines {
        bob_session
            .apply_message_json_at(&line, 100.0)
            .expect("the real server handshake should initialize membership");
    }
    let alice_join_fanout = runtime
        .handle_line_fanout("alice-transport", &hello("alice", "room", true, false))
        .expect("reporting participant should negotiate over the wire");
    for line in alice_join_fanout
        .into_iter()
        .filter(|line| line.client_id == "bob-transport")
    {
        bob_session
            .apply_message_json_at(&line.line, 100.0)
            .expect("the real join fanout should establish observer membership");
    }
    acknowledge_current_server_counter(&mut runtime, "bob-transport");

    let playback_before_status = (
        bob_session.local_position_seconds(),
        bob_session.local_paused(),
    );
    for (index, player_connection, phase) in [
        (
            1_u64,
            ParticipantPlayerConnection::Unavailable,
            ParticipantPlaybackPhase::Empty,
        ),
        (
            2,
            ParticipantPlayerConnection::Starting,
            ParticipantPlaybackPhase::Loading,
        ),
        (
            3,
            ParticipantPlayerConnection::Disconnected,
            ParticipantPlaybackPhase::Unknown,
        ),
        (
            4,
            ParticipantPlayerConnection::Failed,
            ParticipantPlaybackPhase::Failed,
        ),
    ] {
        let report_line =
            encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_participant_status_v1(
                    ParticipantStatusStateExtension::new().with_report(
                        ParticipantStatusReport::new(index, player_connection, phase),
                    ),
                ),
            ))
            .unwrap();
        runtime
            .handle_line("alice-transport", &report_line)
            .expect("the lifecycle report should traverse the real server decoder");

        let now_seconds = 100.0 + index as f64 / 10.0;
        let snapshot_state = runtime
            .periodic_state_sync_message_for_client_at(
                "bob-transport",
                42.0,
                true,
                Some("alice"),
                now_seconds,
            )
            .expect("the observer should receive the lifecycle snapshot");
        bob_session
            .apply_message_json_at(&encode_message_line(&snapshot_state).unwrap(), now_seconds)
            .expect("ClientSession should accept the real lifecycle snapshot");

        let alice = bob_session
            .user_participant_status_at("alice", now_seconds)
            .expect("the lifecycle row should remain visible and current");
        assert_eq!(alice.freshness, ClientParticipantStatusFreshness::Fresh);
        assert_eq!(alice.status.player_connection, Some(player_connection));
        assert_eq!(alice.status.phase, Some(phase));
        assert_eq!(alice.status.sample_age_ms, None);
        assert_eq!(alice.status.position_seconds, None);
        assert_eq!(
            (
                bob_session.local_position_seconds(),
                bob_session.local_paused(),
            ),
            playback_before_status,
            "lifecycle telemetry must remain advisory to canonical playback"
        );
    }
}

#[test]
fn ingestion_binds_identity_rejects_forged_snapshots_and_strictly_validates_reports() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &hello("alice", "room", true, false))
        .unwrap();
    runtime
        .handle_line("legacy-client", &hello("legacy", "room", false, false))
        .unwrap();

    send_status(
        &mut runtime,
        "legacy-client",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Failed)),
    );
    assert!(
        !runtime
            .client_participant_status
            .contains_key("legacy-client")
    );

    send_status(
        &mut runtime,
        "alice-client",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(0, ParticipantPlaybackPhase::Playing)),
    );
    assert!(
        !runtime
            .client_participant_status
            .contains_key("alice-client")
    );

    let valid = status_report(5, ParticipantPlaybackPhase::Playing)
        .with_position_seconds(PARTICIPANT_STATUS_MAX_POSITION_SECONDS)
        .with_playback_rate(PARTICIPANT_STATUS_MAX_PLAYBACK_RATE)
        .with_buffered_ahead_seconds(PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS)
        .with_cache_percent(100.0)
        .with_sample_age_ms(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS)
        .with_position_sample_age_ms(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS);
    let mut forged = ParticipantStatusView::new(ParticipantStatusAvailability::Fresh);
    forged.player_connection = Some(ParticipantPlayerConnection::Failed);
    forged.phase = Some(ParticipantPlaybackPhase::Failed);
    let forged_snapshot =
        ParticipantStatusSnapshot::new(999, BTreeMap::from([("mallory".to_owned(), forged)]));
    send_status(
        &mut runtime,
        "alice-client",
        ParticipantStatusStateExtension::new()
            .with_report(valid)
            .with_snapshot(forged_snapshot),
    );

    let retained = &runtime.client_participant_status["alice-client"];
    assert_eq!(retained.report.report_sequence, 5);
    assert_eq!(retained.username, "alice");
    assert_eq!(retained.room, "room");
    assert_eq!(runtime.client_participant_status.len(), 1);

    for (case, report) in [
        (
            "negative position",
            status_report(6, ParticipantPlaybackPhase::Failed).with_position_seconds(-1.0),
        ),
        (
            "oversized position",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_position_seconds(PARTICIPANT_STATUS_MAX_POSITION_SECONDS + 1.0),
        ),
        (
            "oversized buffer",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_buffered_ahead_seconds(PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS + 1.0),
        ),
        (
            "oversized cache",
            status_report(6, ParticipantPlaybackPhase::Failed).with_cache_percent(100.1),
        ),
        (
            "zero rate",
            status_report(6, ParticipantPlaybackPhase::Failed).with_playback_rate(0.0),
        ),
        (
            "oversized rate",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_playback_rate(PARTICIPANT_STATUS_MAX_PLAYBACK_RATE + 0.1),
        ),
        (
            "oversized sample age",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_sample_age_ms(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS + 1),
        ),
        (
            "oversized position sample age",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_position_seconds(1.0)
                .with_sample_age_ms(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS)
                .with_position_sample_age_ms(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS + 1),
        ),
        (
            "position sample newer than report-wide oldest evidence",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_position_seconds(1.0)
                .with_sample_age_ms(9)
                .with_position_sample_age_ms(10),
        ),
        (
            "zero media generation",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_playback_scope(ParticipantPlaybackScope::new(0)),
        ),
        (
            "zero state revision",
            status_report(6, ParticipantPlaybackPhase::Failed)
                .with_playback_scope(ParticipantPlaybackScope::new(1).with_state_revision(0)),
        ),
    ] {
        send_status(
            &mut runtime,
            "alice-client",
            ParticipantStatusStateExtension::new().with_report(report),
        );
        assert_eq!(
            runtime.client_participant_status["alice-client"]
                .report
                .report_sequence,
            5,
            "{case} must not replace state or consume its sequence"
        );
    }
    runtime
        .handle_state(
            "alice-client",
            crate::ServerStateCommand {
                participant_status: Some(
                    ParticipantStatusStateExtension::new().with_report(
                        status_report(6, ParticipantPlaybackPhase::Failed)
                            .with_buffered_ahead_seconds(f64::NAN),
                    ),
                ),
                ..crate::ServerStateCommand::default()
            },
        )
        .expect("non-finite in-memory DTO should be handled safely");
    assert_eq!(
        runtime.client_participant_status["alice-client"]
            .report
            .report_sequence,
        5,
        "non-finite in-memory telemetry must be rejected"
    );

    for sequence in [5, 4] {
        send_status(
            &mut runtime,
            "alice-client",
            ParticipantStatusStateExtension::new()
                .with_report(status_report(sequence, ParticipantPlaybackPhase::Failed)),
        );
    }
    assert_eq!(
        runtime.client_participant_status["alice-client"]
            .report
            .phase,
        ParticipantPlaybackPhase::Playing
    );

    send_status(
        &mut runtime,
        "alice-client",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(6, ParticipantPlaybackPhase::Seeking)),
    );
    assert_eq!(
        runtime.client_participant_status["alice-client"]
            .report
            .report_sequence,
        6
    );
}

#[test]
fn periodic_snapshot_is_complete_room_scoped_and_explicit_about_capability() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username, room, capable) in [
        ("alice", "alice", "room-a", true),
        ("bob", "bob", "room-a", true),
        ("legacy", "legacy", "room-a", false),
        ("carol", "carol", "room-b", true),
    ] {
        runtime
            .handle_line(client_id, &hello(username, room, capable, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            status_report(1, ParticipantPlaybackPhase::Playing)
                .with_position_seconds(12.5)
                .with_timeline_kind(ParticipantTimelineKind::Vod),
        ),
    );
    send_status(
        &mut runtime,
        "carol",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Seeking)),
    );

    let first = periodic_snapshot(&mut runtime, "bob", 102.5).unwrap();
    assert_eq!(first.mode, ParticipantStatusSnapshotMode::Full);
    assert_eq!(
        first.participants.keys().cloned().collect::<Vec<_>>(),
        ["alice", "bob", "legacy"]
    );
    assert_eq!(
        first.participants["alice"].availability,
        ParticipantStatusAvailability::Fresh
    );
    assert_eq!(first.participants["alice"].report_age_ms, Some(2_500));
    assert_eq!(
        first.participants["bob"].availability,
        ParticipantStatusAvailability::AwaitingReport
    );
    assert_eq!(
        first.participants["legacy"].availability,
        ParticipantStatusAvailability::Unsupported
    );
    assert!(!first.participants.contains_key("carol"));
    assert!(periodic_snapshot(&mut runtime, "legacy", 102.5).is_none());

    runtime.set_time_now_override_seconds(Some(103.0));
    runtime
        .handle_line("dave", &hello("dave", "room-a", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "dave");
    let second = periodic_snapshot(&mut runtime, "dave", 104.0).unwrap();
    assert!(second.revision > first.revision);
    assert_eq!(
        second.participants.keys().cloned().collect::<Vec<_>>(),
        ["alice", "bob", "dave", "legacy"]
    );
    assert_eq!(
        second.participants["dave"].availability,
        ParticipantStatusAvailability::AwaitingReport
    );
}

#[test]
fn report_received_at_snapshot_time_has_zero_age_and_remains_fresh() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );

    let snapshot = periodic_snapshot(&mut runtime, "alice", 100.0).unwrap();
    assert_eq!(
        snapshot.participants["alice"].availability,
        ParticipantStatusAvailability::Fresh
    );
    assert_eq!(snapshot.participants["alice"].report_age_ms, Some(0));
}

#[test]
fn freshness_projection_and_server_derived_offsets_require_strict_correlation() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, true))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let generation = start_new_media(&mut runtime, "alice", 1);
    acknowledge_current_server_counter(&mut runtime, "alice");
    acknowledge_current_server_counter(&mut runtime, "bob");
    {
        let barrier = runtime.room_playback_barriers.get_mut("room").unwrap();
        barrier.state_revision = Some(19);
        barrier.phase = PlaybackBarrierPhase::Committed;
    }
    runtime.room_playback_states.insert(
        "room".to_owned(),
        RoomPlaybackState {
            position: 100.0,
            paused: false,
            set_by: Some("alice".to_owned()),
            updated_at_seconds: 100.0,
        },
    );
    let transport_revision = runtime.room_participant_status_scopes["room"].transport_revision;
    fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            scoped_vod_report(
                1,
                generation,
                19,
                transport_revision,
                ParticipantPlaybackPhase::Playing,
                98.0,
            )
            .with_sample_age_ms(250)
            .with_position_sample_age_ms(250),
        ),
    );
    let fresh = periodic_snapshot(&mut runtime, "bob", 101.0).unwrap();
    let alice = &fresh.participants["alice"];
    assert_eq!(alice.availability, ParticipantStatusAvailability::Fresh);
    assert_eq!(alice.sample_age_ms, Some(1_375));
    assert_eq!(alice.position_sample_age_ms, Some(1_375));
    assert_eq!(alice.position_seconds, Some(99.375));
    assert_eq!(alice.room_offset_seconds, Some(-1.625));

    let (delayed, delayed_transitions) = capture_server_lifecycle_transitions(|| {
        periodic_snapshot(&mut runtime, "bob", 104.0).unwrap()
    });
    assert_eq!(
        delayed.participants["alice"].availability,
        ParticipantStatusAvailability::Delayed
    );
    assert_eq!(delayed.participants["alice"].room_offset_seconds, None);
    let delayed_transition = delayed_transitions
        .iter()
        .find(|transition| transition.transition == "STATUS-DELAY-001")
        .expect("fresh-to-delayed projection should emit its lifecycle boundary");
    assert_eq!(delayed_transition.trigger, Trigger::Timer);
    assert_eq!(delayed_transition.disposition, Disposition::Applied);
    assert!(
        delayed_transition
            .identities
            .contains(&("report-sequence", 1))
    );

    let (stale, stale_transitions) = capture_server_lifecycle_transitions(|| {
        periodic_snapshot(&mut runtime, "bob", 111.0).unwrap()
    });
    let alice = &stale.participants["alice"];
    assert_eq!(alice.availability, ParticipantStatusAvailability::Stale);
    assert_eq!(
        alice.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(alice.phase, None);
    assert_eq!(alice.position_seconds, None);
    assert_eq!(alice.buffered_ahead_seconds, None);
    assert_eq!(alice.room_offset_seconds, None);
    let stale_transition = stale_transitions
        .iter()
        .find(|transition| transition.transition == "STATUS-STALE-001")
        .expect("delayed-to-stale projection should emit its lifecycle boundary");
    assert_eq!(stale_transition.trigger, Trigger::Timer);
    assert_eq!(stale_transition.disposition, Disposition::Applied);
    assert!(
        stale_transition
            .identities
            .contains(&("report-sequence", 1))
    );

    runtime.set_time_now_override_seconds(Some(120.0));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(scoped_vod_report(
            2,
            generation,
            18,
            transport_revision,
            ParticipantPlaybackPhase::Playing,
            118.0,
        )),
    );
    let wrong_revision = periodic_snapshot(&mut runtime, "bob", 120.1).unwrap();
    assert_eq!(
        wrong_revision.participants["alice"].availability,
        ParticipantStatusAvailability::Fresh
    );
    assert_eq!(
        wrong_revision.participants["alice"].correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(
        wrong_revision.participants["alice"].phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(wrong_revision.participants["alice"].position_seconds, None);

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(scoped_vod_report(
            3,
            generation,
            19,
            transport_revision,
            ParticipantPlaybackPhase::Seeking,
            118.0,
        )),
    );
    assert_eq!(
        periodic_snapshot(&mut runtime, "bob", 120.2)
            .unwrap()
            .participants["alice"]
            .room_offset_seconds,
        None
    );

    let mut live = scoped_vod_report(
        4,
        generation,
        19,
        transport_revision,
        ParticipantPlaybackPhase::Playing,
        118.0,
    );
    live.timeline_kind = ParticipantTimelineKind::Live;
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(live),
    );
    assert_eq!(
        periodic_snapshot(&mut runtime, "bob", 120.3)
            .unwrap()
            .participants["alice"]
            .room_offset_seconds,
        None
    );

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            scoped_vod_report(
                5,
                generation,
                19,
                transport_revision,
                ParticipantPlaybackPhase::Playing,
                118.0,
            )
            .with_sample_age_ms(3_000),
        ),
    );
    assert_eq!(
        periodic_snapshot(&mut runtime, "bob", 120.1)
            .unwrap()
            .participants["alice"]
            .room_offset_seconds,
        None,
        "a stale underlying player sample must not produce a precise offset"
    );

    for (sequence, mut report, reason) in [
        (
            6,
            scoped_vod_report(
                6,
                generation,
                19,
                transport_revision,
                ParticipantPlaybackPhase::Playing,
                118.0,
            ),
            "missing sample age",
        ),
        (
            7,
            scoped_vod_report(
                7,
                generation,
                19,
                transport_revision,
                ParticipantPlaybackPhase::Playing,
                118.0,
            ),
            "missing cache-pause evidence",
        ),
        (
            8,
            scoped_vod_report(
                8,
                generation,
                19,
                transport_revision,
                ParticipantPlaybackPhase::Playing,
                118.0,
            ),
            "missing playback rate",
        ),
    ] {
        match sequence {
            6 => report.sample_age_ms = None,
            7 => report.paused_for_cache = None,
            8 => report.playback_rate = None,
            _ => unreachable!(),
        }
        send_status(
            &mut runtime,
            "alice",
            ParticipantStatusStateExtension::new().with_report(report),
        );
        let view = periodic_snapshot(&mut runtime, "bob", 120.4 + sequence as f64 / 100.0)
            .unwrap()
            .participants
            .remove("alice")
            .unwrap();
        assert_eq!(view.room_offset_seconds, None, "{reason} must fail closed");
        if reason == "missing sample age" {
            assert_eq!(view.availability, ParticipantStatusAvailability::Fresh);
            assert_eq!(view.phase, Some(ParticipantPlaybackPhase::Playing));
            assert!(
                view.position_seconds.is_some(),
                "position uses its own explicit evidence clock"
            );
            assert_eq!(view.playback_rate, None);
            assert_eq!(view.buffered_ahead_seconds, None);
        }
    }

    let mut missing_position_age = scoped_vod_report(
        9,
        generation,
        19,
        transport_revision,
        ParticipantPlaybackPhase::ReadyPaused,
        118.0,
    );
    missing_position_age.logical_paused = Some(true);
    missing_position_age.position_sample_age_ms = None;
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(missing_position_age),
    );
    let missing_position_age = periodic_snapshot(&mut runtime, "bob", 120.5)
        .unwrap()
        .participants
        .remove("alice")
        .unwrap();
    assert_eq!(missing_position_age.position_seconds, None);
    assert_eq!(missing_position_age.position_sample_age_ms, None);
    assert_eq!(missing_position_age.room_offset_seconds, None);

    runtime.set_time_now_override_seconds(Some(200.0));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(scoped_vod_report(
            10,
            generation,
            19,
            transport_revision,
            ParticipantPlaybackPhase::Playing,
            198.0,
        )),
    );
    let rollback = periodic_snapshot(&mut runtime, "bob", 199.0).unwrap();
    assert_eq!(
        rollback.participants["alice"].availability,
        ParticipantStatusAvailability::Stale,
        "a wall-clock rollback must never rejuvenate a retained report"
    );
    let caught_up = periodic_snapshot(&mut runtime, "bob", 200.1).unwrap();
    assert_eq!(
        caught_up.participants["alice"].availability,
        ParticipantStatusAvailability::Stale,
        "the same report must stay stale when wall time catches up"
    );

    runtime
        .handle_line("alice", &hello("alice", "room", true, true))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", 200.0),
        None,
        "a fresh connection has no trustworthy delay evidence"
    );
    fixture_issued_ping_echo(&mut runtime, "alice", 0.0, 0.0);
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", 200.0),
        None,
        "an unbounded but finite ping estimate must not become status timing evidence"
    );
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(scoped_vod_report(
            1,
            generation,
            19,
            transport_revision,
            ParticipantPlaybackPhase::Playing,
            198.0,
        )),
    );
    let untrusted_delay = periodic_snapshot(&mut runtime, "bob", 200.1)
        .unwrap()
        .participants
        .remove("alice")
        .unwrap();
    assert!(untrusted_delay.position_seconds.is_some());
    assert_eq!(
        untrusted_delay.room_offset_seconds, None,
        "precise room offset requires a bounded server-owned delay estimate"
    );
}

#[test]
fn fresh_position_clock_survives_stale_unrelated_evidence_through_server_projection() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, true))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let generation = start_new_media(&mut runtime, "alice", 81);
    acknowledge_current_server_counter(&mut runtime, "alice");
    acknowledge_current_server_counter(&mut runtime, "bob");
    {
        let barrier = runtime.room_playback_barriers.get_mut("room").unwrap();
        barrier.state_revision = Some(1);
        barrier.phase = PlaybackBarrierPhase::Committed;
    }
    let scope = runtime.room_participant_status_scopes["room"];

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            scoped_vod_report(
                1,
                generation,
                1,
                scope.transport_revision,
                ParticipantPlaybackPhase::Playing,
                42.5,
            )
            .with_sample_age_ms(11_000)
            .with_position_sample_age_ms(0),
        ),
    );

    let view = periodic_snapshot(&mut runtime, "bob", 100.1)
        .unwrap()
        .participants
        .remove("alice")
        .unwrap();
    assert_eq!(view.availability, ParticipantStatusAvailability::Fresh);
    assert_eq!(view.correlation, Some(ParticipantStatusCorrelation::Exact));
    assert_eq!(view.position_sample_age_ms, Some(100));
    assert_eq!(view.position_seconds, Some(42.6));
    assert_eq!(view.sample_age_ms, Some(11_100));
    assert_eq!(view.playback_rate, None);
    assert_eq!(view.buffered_ahead_seconds, None);
    assert_eq!(view.room_offset_seconds, None);
}

#[test]
fn precise_offset_phase_gate_distinguishes_paused_rebuffering_and_unsafe_evidence() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, true))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let generation = start_new_media(&mut runtime, "alice", 73);
    acknowledge_current_server_counter(&mut runtime, "alice");
    acknowledge_current_server_counter(&mut runtime, "bob");
    {
        let barrier = runtime.room_playback_barriers.get_mut("room").unwrap();
        barrier.state_revision = Some(19);
        barrier.phase = PlaybackBarrierPhase::Committed;
    }
    runtime.room_playback_states.insert(
        "room".to_owned(),
        RoomPlaybackState {
            position: 100.0,
            paused: false,
            set_by: Some("alice".to_owned()),
            updated_at_seconds: 100.0,
        },
    );
    let scope = runtime.room_participant_status_scopes["room"];
    fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);
    let view_for = |runtime: &mut ServerRuntime, report: ParticipantStatusReport| {
        send_status(
            runtime,
            "alice",
            ParticipantStatusStateExtension::new().with_report(report),
        );
        periodic_snapshot(runtime, "bob", 100.0)
            .unwrap()
            .participants
            .remove("alice")
            .unwrap()
    };

    let ready_paused = view_for(
        &mut runtime,
        scoped_vod_report(
            1,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::ReadyPaused,
            98.0,
        )
        .with_logical_paused(true),
    );
    assert!(
        ready_paused.room_offset_seconds.is_some(),
        "connected ReadyPaused with explicit pause evidence supports a precise offset"
    );

    let wrong_logical_pause = view_for(
        &mut runtime,
        scoped_vod_report(
            2,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::ReadyPaused,
            98.0,
        ),
    );
    assert_eq!(wrong_logical_pause.room_offset_seconds, None);

    let cache_paused_ready = view_for(
        &mut runtime,
        scoped_vod_report(
            3,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::ReadyPaused,
            98.0,
        )
        .with_logical_paused(true)
        .with_paused_for_cache(true),
    );
    assert_eq!(cache_paused_ready.room_offset_seconds, None);

    let rebuffering = view_for(
        &mut runtime,
        scoped_vod_report(
            4,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::Rebuffering,
            98.0,
        )
        .with_paused_for_cache(true),
    );
    assert!(
        rebuffering.room_offset_seconds.is_some(),
        "connected Rebuffering with cache-pause evidence supports a precise offset"
    );

    let unconfirmed_rebuffering = view_for(
        &mut runtime,
        scoped_vod_report(
            5,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::Rebuffering,
            98.0,
        ),
    );
    assert_eq!(unconfirmed_rebuffering.room_offset_seconds, None);

    let mut disconnected_rebuffering = scoped_vod_report(
        6,
        generation,
        19,
        scope.transport_revision,
        ParticipantPlaybackPhase::Rebuffering,
        98.0,
    )
    .with_paused_for_cache(true);
    disconnected_rebuffering.player_connection = ParticipantPlayerConnection::Disconnected;
    assert_eq!(
        view_for(&mut runtime, disconnected_rebuffering).room_offset_seconds,
        None,
        "disconnected player evidence must never produce a precise offset"
    );

    let oversized_offset = view_for(
        &mut runtime,
        scoped_vod_report(
            7,
            generation,
            19,
            scope.transport_revision,
            ParticipantPlaybackPhase::ReadyPaused,
            100_000.0,
        )
        .with_logical_paused(true),
    );
    assert_eq!(
        oversized_offset.room_offset_seconds, None,
        "finite offsets beyond the safety bound must fail closed"
    );
}

#[test]
fn participant_status_rejects_invalid_forward_delay_without_reviving_it() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001, 90.001] {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime
            .handle_line("alice", &hello("alice", "room", true, false))
            .unwrap();
        fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);
        runtime
            .client_state_counters
            .get_mut("alice")
            .unwrap()
            .ping_forward_delay_seconds = invalid;
        assert_eq!(
            runtime.participant_status_forward_delay_ms_at("alice", 100.0),
            None
        );
        runtime
            .client_state_counters
            .get_mut("alice")
            .unwrap()
            .ping_forward_delay_seconds = 0.125;
        assert_eq!(
            runtime.participant_status_forward_delay_ms_at("alice", 100.0),
            None
        );
    }
}

#[test]
fn participant_status_forward_delay_evidence_has_a_bounded_lifetime() {
    let mut runtime = ServerRuntime::default();
    let observed_at_seconds = 100.0;
    runtime.set_time_now_override_seconds(Some(observed_at_seconds));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);

    let expires_at_seconds = observed_at_seconds + PROTOCOL_TIMEOUT_SECONDS;
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", expires_at_seconds),
        Some(125),
        "delay evidence remains valid through the protocol-timeout boundary"
    );
    let after_expiry_seconds = expires_at_seconds + 0.001;
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", after_expiry_seconds),
        None,
        "delay evidence must expire after the protocol timeout"
    );

    runtime.set_time_now_override_seconds(Some(after_expiry_seconds));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    assert_eq!(
        runtime.client_participant_status["alice"].forward_delay_ms, None,
        "an expired estimate must not be retained as report timing evidence"
    );
}

#[test]
fn participant_status_forward_delay_evidence_fails_closed_on_clock_rollback() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);

    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", 99.999),
        None,
        "backward time must invalidate rather than rejuvenate delay evidence"
    );

    runtime.set_time_now_override_seconds(Some(99.999));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    assert_eq!(
        runtime.client_participant_status["alice"].forward_delay_ms, None,
        "a report received after clock rollback must retain no delay evidence"
    );
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", 100.0),
        None,
        "pre-rollback ping evidence must not revive when wall time catches up"
    );

    runtime.set_time_now_override_seconds(Some(100.0));
    fixture_issued_ping_echo(&mut runtime, "alice", 99.75, 0.25);
    assert_eq!(
        runtime.participant_status_forward_delay_ms_at("alice", 100.0),
        Some(125),
        "a genuinely new ping observation may establish fresh delay evidence"
    );
}

#[test]
fn lifecycle_and_phase_incompatible_media_evidence_is_redacted_before_projection() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let scope = runtime.room_participant_status_scopes["room"].to_wire(None);

    for (sequence, connection, phase) in [
        (
            1,
            ParticipantPlayerConnection::Connected,
            ParticipantPlaybackPhase::Empty,
        ),
        (
            2,
            ParticipantPlayerConnection::Connected,
            ParticipantPlaybackPhase::Loading,
        ),
        (
            3,
            ParticipantPlayerConnection::Connected,
            ParticipantPlaybackPhase::Seeking,
        ),
        (
            4,
            ParticipantPlayerConnection::Connected,
            ParticipantPlaybackPhase::Failed,
        ),
        (
            5,
            ParticipantPlayerConnection::Starting,
            ParticipantPlaybackPhase::Loading,
        ),
        (
            6,
            ParticipantPlayerConnection::Disconnected,
            ParticipantPlaybackPhase::Rebuffering,
        ),
        (
            7,
            ParticipantPlayerConnection::Failed,
            ParticipantPlaybackPhase::Failed,
        ),
    ] {
        let mut report = status_report(sequence, phase)
            .with_playback_scope(scope)
            .with_timeline_kind(ParticipantTimelineKind::Vod)
            .with_position_seconds(40.0)
            .with_logical_paused(false)
            .with_playback_rate(1.0)
            .with_paused_for_cache(true)
            .with_cache_percent(50.0)
            .with_buffered_ahead_seconds(8.0)
            .with_sample_age_ms(0)
            .with_position_sample_age_ms(0);
        report.player_connection = connection;
        send_status(
            &mut runtime,
            "alice",
            ParticipantStatusStateExtension::new().with_report(report),
        );
        let view = periodic_snapshot(&mut runtime, "bob", 10.1)
            .unwrap()
            .participants
            .remove("alice")
            .unwrap();
        assert_eq!(view.player_connection, Some(connection));
        assert_eq!(view.phase, Some(phase));
        assert_eq!(view.position_seconds, None, "phase {phase:?}");
        assert_eq!(view.position_sample_age_ms, None, "phase {phase:?}");
        assert_eq!(view.buffered_ahead_seconds, None, "phase {phase:?}");
        assert_eq!(view.cache_percent, None, "phase {phase:?}");
        assert_eq!(view.room_offset_seconds, None, "phase {phase:?}");
    }

    let prebuffering = status_report(8, ParticipantPlaybackPhase::Prebuffering)
        .with_playback_scope(scope)
        .with_timeline_kind(ParticipantTimelineKind::Vod)
        .with_position_seconds(40.0)
        .with_paused_for_cache(true)
        .with_cache_percent(50.0)
        .with_buffered_ahead_seconds(8.0)
        .with_sample_age_ms(0)
        .with_position_sample_age_ms(0);
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(prebuffering),
    );
    let prebuffering = periodic_snapshot(&mut runtime, "bob", 10.1)
        .unwrap()
        .participants
        .remove("alice")
        .unwrap();
    assert_eq!(prebuffering.position_seconds, None);
    assert_eq!(prebuffering.buffered_ahead_seconds, Some(8.0));
}

#[test]
fn uncorrelated_loading_and_disconnect_keep_coarse_truth_and_age_normally() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Loading)),
    );
    let loading = periodic_snapshot(&mut runtime, "bob", 10.1).unwrap();
    let loading = &loading.participants["alice"];
    assert_eq!(loading.availability, ParticipantStatusAvailability::Fresh);
    assert_eq!(
        loading.correlation,
        Some(ParticipantStatusCorrelation::Uncorrelated)
    );
    assert_eq!(loading.phase, Some(ParticipantPlaybackPhase::Loading));
    assert_eq!(
        loading.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(loading.position_seconds, None);

    runtime.set_time_now_override_seconds(Some(11.0));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            ParticipantStatusReport::new(
                2,
                ParticipantPlayerConnection::Disconnected,
                ParticipantPlaybackPhase::Rebuffering,
            )
            .with_sample_age_ms(0),
        ),
    );
    let disconnected = periodic_snapshot(&mut runtime, "bob", 11.1).unwrap();
    let disconnected = &disconnected.participants["alice"];
    assert_eq!(
        disconnected.player_connection,
        Some(ParticipantPlayerConnection::Disconnected)
    );
    assert_eq!(
        disconnected.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );
    assert_eq!(disconnected.position_seconds, None);

    let stale = periodic_snapshot(&mut runtime, "bob", 22.0).unwrap();
    assert_eq!(
        stale.participants["alice"].availability,
        ParticipantStatusAvailability::Stale
    );
    assert_eq!(stale.participants["alice"].phase, None);
    assert_eq!(
        stale.participants["alice"].player_connection,
        Some(ParticipantPlayerConnection::Disconnected)
    );
}

#[test]
fn uncorrelated_playing_report_keeps_local_timestamp_but_never_room_offset() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }

    let report = status_report(1, ParticipantPlaybackPhase::Playing)
        .with_timeline_kind(ParticipantTimelineKind::Vod)
        .with_position_seconds(42.5)
        .with_logical_paused(false)
        .with_playback_rate(1.0)
        .with_paused_for_cache(false)
        .with_cache_percent(50.0)
        .with_buffered_ahead_seconds(8.0)
        .with_sample_age_ms(0)
        .with_position_sample_age_ms(0);
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(report),
    );

    let snapshot = periodic_snapshot(&mut runtime, "bob", 10.1).unwrap();
    let alice = &snapshot.participants["alice"];
    assert_eq!(
        alice.correlation,
        Some(ParticipantStatusCorrelation::Uncorrelated)
    );
    assert!(alice.position_seconds.is_some());
    assert_eq!(alice.logical_paused, Some(false));
    assert_eq!(alice.playback_rate, Some(1.0));
    assert_eq!(alice.buffered_ahead_seconds, Some(8.0));
    assert_eq!(alice.room_offset_seconds, None);
}

#[test]
fn partial_matching_scope_is_uncorrelated_while_explicit_conflict_is_superseded() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let current_scope = runtime.room_participant_status_scopes["room"].to_wire(None);
    let partial_scope = ParticipantPlaybackScope::new(current_scope.media_generation);
    let report = status_report(1, ParticipantPlaybackPhase::Playing)
        .with_playback_scope(partial_scope)
        .with_timeline_kind(ParticipantTimelineKind::Vod)
        .with_position_seconds(42.5)
        .with_logical_paused(false)
        .with_playback_rate(1.0)
        .with_paused_for_cache(false)
        .with_sample_age_ms(0)
        .with_position_sample_age_ms(0);
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(report),
    );
    let partial = periodic_snapshot(&mut runtime, "bob", 10.1).unwrap();
    assert_eq!(
        partial.participants["alice"].correlation,
        Some(ParticipantStatusCorrelation::Uncorrelated)
    );
    assert!(partial.participants["alice"].position_seconds.is_some());
    assert_eq!(partial.participants["alice"].room_offset_seconds, None);

    runtime.set_time_now_override_seconds(Some(11.0));
    let conflicting_scope = ParticipantPlaybackScope::new(current_scope.media_generation)
        .with_transport_revision(current_scope.transport_revision.unwrap() + 1);
    let conflicting = status_report(2, ParticipantPlaybackPhase::Playing)
        .with_playback_scope(conflicting_scope)
        .with_timeline_kind(ParticipantTimelineKind::Vod)
        .with_position_seconds(55.0)
        .with_sample_age_ms(0)
        .with_position_sample_age_ms(0);
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(conflicting),
    );
    let superseded = periodic_snapshot(&mut runtime, "bob", 11.1).unwrap();
    assert_eq!(
        superseded.participants["alice"].correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(superseded.participants["alice"].position_seconds, None);
}

#[test]
fn canonical_seek_advances_transport_scope_and_supersedes_old_media_evidence() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }
    let scope = runtime.room_participant_status_scopes["room"];
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            status_report(1, ParticipantPlaybackPhase::ReadyPaused)
                .with_playback_scope(scope.to_wire(None))
                .with_timeline_kind(ParticipantTimelineKind::Vod)
                .with_position_seconds(10.0)
                .with_logical_paused(true)
                .with_paused_for_cache(false)
                .with_sample_age_ms(0),
        ),
    );
    assert_eq!(
        periodic_snapshot(&mut runtime, "bob", 10.1)
            .unwrap()
            .participants["alice"]
            .correlation,
        Some(ParticipantStatusCorrelation::Exact)
    );

    runtime
        .handle_protocol_message(
            "bob",
            ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(20.0)
                        .with_paused(true)
                        .with_do_seek(true),
                ),
            ),
        )
        .unwrap();
    let advanced = runtime.room_participant_status_scopes["room"];
    assert!(advanced.transport_revision > scope.transport_revision);
    acknowledge_current_server_counter(&mut runtime, "bob");
    let superseded = periodic_snapshot(&mut runtime, "bob", 11.2).unwrap();
    let alice = &superseded.participants["alice"];
    assert_eq!(
        alice.correlation,
        Some(ParticipantStatusCorrelation::Superseded)
    );
    assert_eq!(alice.phase, Some(ParticipantPlaybackPhase::ReadyPaused));
    assert_eq!(alice.position_seconds, None);
    assert_eq!(alice.room_offset_seconds, None);
}

#[test]
fn status_is_advisory_and_does_not_suppress_bundled_canonical_playstate() {
    let mut with_status = ServerRuntime::default();
    let mut without_status = ServerRuntime::default();
    for runtime in [&mut with_status, &mut without_status] {
        runtime.set_time_now_override_seconds(Some(10.0));
        runtime
            .handle_line("alice", &hello("alice", "room", true, false))
            .unwrap();
    }
    let playstate = PlaystatePayload::new()
        .with_position(321.5)
        .with_paused(false)
        .with_do_seek(false);
    without_status
        .handle_protocol_message(
            "alice",
            ProtocolMessage::state(StatePayload::new().with_playstate(playstate.clone())),
        )
        .unwrap();
    with_status
        .handle_protocol_message(
            "alice",
            ProtocolMessage::state(
                StatePayload::new()
                    .with_playstate(playstate)
                    .with_participant_status_v1(
                        ParticipantStatusStateExtension::new().with_report(
                            status_report(1, ParticipantPlaybackPhase::Failed)
                                .with_position_seconds(PARTICIPANT_STATUS_MAX_POSITION_SECONDS),
                        ),
                    ),
            ),
        )
        .unwrap();

    assert_eq!(
        with_status.room_playback_states,
        without_status.room_playback_states
    );
    assert_eq!(
        with_status.client_playback_states,
        without_status.client_playback_states
    );
    assert!(with_status.client_participant_status.contains_key("alice"));
    assert!(without_status.client_participant_status.is_empty());
    assert!(with_status.room_readiness.is_empty());
    assert!(with_status.room_playback_barriers.is_empty());
}

#[test]
fn lifecycle_boundaries_clear_status_and_reset_connection_sequence() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice", &hello("alice", "room-a", true, false))
        .unwrap();
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(9, ParticipantPlaybackPhase::Playing)),
    );

    runtime
        .handle_line("alice", r#"{"Set":{"room":{"name":"room-b"}}}"#)
        .unwrap();
    assert!(!runtime.client_participant_status.contains_key("alice"));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Loading)),
    );
    assert!(!runtime.client_participant_status.contains_key("alice"));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(10, ParticipantPlaybackPhase::Loading)),
    );
    assert_eq!(
        runtime.client_participant_status["alice"]
            .report
            .report_sequence,
        10
    );

    runtime
        .handle_line(
            "alice",
            &json!({ "Set": { "features": { SOROTTE_PARTICIPANT_STATUS_V1: false } } }).to_string(),
        )
        .unwrap();
    assert!(!runtime.client_participant_status.contains_key("alice"));
    runtime
        .handle_line(
            "alice",
            &json!({ "Set": { "features": { SOROTTE_PARTICIPANT_STATUS_V1: true } } }).to_string(),
        )
        .unwrap();
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::ReadyPaused)),
    );
    assert!(!runtime.client_participant_status.contains_key("alice"));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(11, ParticipantPlaybackPhase::ReadyPaused)),
    );

    runtime
        .handle_line("alice", &hello("alice", "room-b", true, false))
        .unwrap();
    assert!(!runtime.client_participant_status.contains_key("alice"));
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    runtime.handle_transport_disconnect_fanout("alice").unwrap();
    assert!(!runtime.client_participant_status.contains_key("alice"));
}

#[test]
fn identical_participant_status_feature_heartbeat_is_a_noop() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    runtime
        .handle_line("bob", &hello("bob", "room", true, false))
        .unwrap();
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );

    let unchanged_features = runtime
        .session("alice")
        .expect("alice should remain connected")
        .capabilities
        .to_wire_value();
    let fanout = runtime
        .handle_line_fanout(
            "alice",
            &json!({
                "Set": {
                    "features": unchanged_features
                }
            })
            .to_string(),
        )
        .expect("an unchanged feature heartbeat should be accepted");

    assert!(
        fanout.is_empty(),
        "an identical capability heartbeat must not amplify into room fanout"
    );
    assert_eq!(
        runtime.client_participant_status["alice"]
            .report
            .report_sequence,
        1,
        "an identical capability heartbeat must preserve accepted status"
    );
}

#[test]
fn accepted_new_media_generation_clears_every_old_room_report() {
    let mut runtime = ServerRuntime::default();
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, true))
            .unwrap();
        send_status(
            &mut runtime,
            client_id,
            ParticipantStatusStateExtension::new().with_report(
                status_report(1, ParticipantPlaybackPhase::Playing).with_position_seconds(500.0),
            ),
        );
    }
    assert_eq!(runtime.client_participant_status.len(), 2);

    let generation = start_new_media(&mut runtime, "alice", 41);
    assert_ne!(generation, 0);
    assert!(
        runtime.client_participant_status.is_empty(),
        "old media positions and buffers must disappear at the generation boundary"
    );
}

#[test]
fn zero_authoritative_media_generation_advances_existing_room_scope() {
    let mut runtime = ServerRuntime::default();
    runtime.ensure_room_state("room");
    let before = runtime.room_participant_status_scopes["room"];

    runtime.advance_participant_status_media_generation("room", Some(0));

    let after = runtime.room_participant_status_scopes["room"];
    assert_eq!(
        after.media_generation,
        before.media_generation.saturating_add(1),
        "zero is not an authoritative generation and must advance the current scope"
    );
    assert_eq!(
        after.transport_revision,
        before.transport_revision.saturating_add(1),
        "every media-generation boundary must also advance transport revision"
    );
}

#[test]
fn policy_only_barrier_replacement_advances_status_scope_before_dropping_state_revision() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, true))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
    }

    let generation = start_new_media(&mut runtime, "alice", 1);
    acknowledge_current_server_counter(&mut runtime, "alice");
    acknowledge_current_server_counter(&mut runtime, "bob");
    {
        let barrier = runtime.room_playback_barriers.get_mut("room").unwrap();
        barrier.state_revision = Some(19);
        barrier.phase = PlaybackBarrierPhase::Complete;
    }
    let old_scope = runtime.room_participant_status_scopes["room"].to_wire(Some(19));
    assert_eq!(old_scope.media_generation, generation);
    assert_eq!(old_scope.state_revision, Some(19));

    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new().with_report(
            status_report(1, ParticipantPlaybackPhase::Playing).with_playback_scope(old_scope),
        ),
    );
    assert_eq!(
        periodic_snapshot(&mut runtime, "bob", 100.0)
            .unwrap()
            .participants["alice"]
            .correlation,
        Some(ParticipantStatusCorrelation::Exact)
    );
    assert!(
        runtime
            .participant_status_snapshot_cache
            .contains_key("room")
    );

    runtime
        .handle_line_fanout(
            "alice",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":2,"requestId":"replacement-policy","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("policy-only new playback should replace the terminal barrier");

    assert!(!runtime.room_playback_barriers.contains_key("room"));
    let new_scope = runtime.room_participant_status_scopes["room"].to_wire(None);
    assert!(
        new_scope.media_generation > old_scope.media_generation,
        "dropping stateRevision must cross an independent media fence"
    );
    assert!(
        new_scope.transport_revision > old_scope.transport_revision,
        "the replacement must also cross the transport-authority fence"
    );
    assert_eq!(new_scope.state_revision, None);
    assert!(!runtime.client_participant_status.contains_key("alice"));
    assert!(
        !runtime
            .participant_status_snapshot_cache
            .contains_key("room")
    );
}

#[test]
fn fenced_connection_replacement_scrubs_old_status_before_disconnect() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true, true))
        .unwrap();
    send_status(
        &mut runtime,
        "alice-old",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":41,"requestId":"status-replacement","loadIntent":"newPlayback","logicalMediaId":"media","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .unwrap();
    send_status(
        &mut runtime,
        "alice-old",
        ParticipantStatusStateExtension::new().with_report(
            status_report(2, ParticipantPlaybackPhase::Playing)
                .with_playback_scope(ParticipantPlaybackScope::new(1)),
        ),
    );
    assert!(
        runtime.client_participant_status.contains_key("alice-old"),
        "the test must retain post-generation status before exercising connection fencing"
    );
    runtime
        .handle_line("alice-new", &hello("alice", "room", true, true))
        .unwrap();
    runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"status-replacement","originalRequestNonce":41,"recoveryNonce":42,"logicalMediaId":"media"}}}}"#,
        )
        .unwrap();

    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
    assert!(!runtime.client_participant_status.contains_key("alice-old"));
    assert!(runtime.sessions.contains_key("alice-old"));
    assert!(runtime.sessions.contains_key("alice-new"));
}

#[test]
fn large_room_snapshots_use_only_the_coalescible_periodic_delivery_path() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(50.0));
    for index in 0..32 {
        let client_id = format!("client-{index:02}");
        let username = format!("user-{index:02}");
        runtime
            .handle_line(&client_id, &hello(&username, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, &client_id);
        let outbound = send_status(
            &mut runtime,
            &client_id,
            ParticipantStatusStateExtension::new().with_report(
                status_report(1, ParticipantPlaybackPhase::Playing)
                    .with_position_seconds(index as f64),
            ),
        );
        assert!(
            outbound.iter().all(|message| {
                let ProtocolMessage::State(message) = message else {
                    return true;
                };
                message
                    .state
                    .participant_status_v1()
                    .ok()
                    .flatten()
                    .and_then(|extension| extension.snapshot)
                    .is_none()
            }),
            "a client report must not trigger immediate N-way status fanout"
        );
    }

    let dispatch = runtime.collect_dispatch_at(51.0).unwrap();
    let status_lines = dispatch
        .outbound_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::State(message) = message else {
                return None;
            };
            message
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.snapshot)
                .map(|snapshot| (line.delivery, snapshot))
        })
        .collect::<Vec<_>>();
    assert_eq!(status_lines.len(), 32);
    assert!(status_lines.iter().all(|(delivery, snapshot)| {
        *delivery == ServerOutboundDelivery::CoalesciblePeriodicState
            && snapshot.participants.len() == 32
    }));
    let revisions = status_lines
        .iter()
        .map(|(_, snapshot)| snapshot.revision)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        revisions.len(),
        1,
        "one same-instant room snapshot revision must be shared by every recipient"
    );
}

#[test]
fn oversized_full_room_snapshot_compacts_below_the_smallest_client_frame_limit() {
    let mut runtime = ServerRuntime::default();
    // This test deliberately exceeds normal fanout capacity to test advisory
    // representation at 600 members. Admission budgets have separate tests.
    runtime
        .set_resource_limits(crate::ServerResourceLimits {
            queued_bytes_total: 1024 * 1024 * 1024,
            ..crate::ServerResourceLimits::default()
        })
        .unwrap();
    runtime.set_time_now_override_seconds(Some(50.0));
    const PARTICIPANTS: usize = 240;
    for index in 0..PARTICIPANTS {
        let client_id = format!("client-{index:03}");
        let username = format!("participant{index:03}");
        runtime
            .handle_line(
                &client_id,
                &hello(&username, "room", true, false).replace(
                    "\"features\":{",
                    "\"features\":{\"sorotteLargeProtocolFramesV1\":true,",
                ),
            )
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, &client_id);
        let scope = runtime.room_participant_status_scopes["room"];
        send_status(
            &mut runtime,
            &client_id,
            ParticipantStatusStateExtension::new().with_report(
                status_report(1, ParticipantPlaybackPhase::Playing)
                    .with_playback_scope(scope.to_wire(None))
                    .with_timeline_kind(ParticipantTimelineKind::Vod)
                    .with_position_seconds(index as f64)
                    .with_logical_paused(false)
                    .with_playback_rate(1.0)
                    .with_paused_for_cache(false)
                    .with_cache_percent(50.0)
                    .with_buffered_ahead_seconds(8.0)
                    .with_sample_age_ms(0),
            ),
        );
    }

    let message = runtime
        .periodic_state_sync_message_for_client_at("client-000", 0.0, true, None, 51.0)
        .expect("large-room recipient should receive a bounded periodic State");
    let encoded = encode_message_line(&message).expect("bounded snapshot should encode");
    assert!(
        encoded.len() <= sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES,
        "participant status must fit the CLI reader limit: {} bytes",
        encoded.len()
    );
    let ProtocolMessage::State(message) = message else {
        panic!("periodic status must be State");
    };
    let snapshot = message
        .state
        .participant_status_v1()
        .unwrap()
        .and_then(|extension| extension.snapshot)
        .expect("bounded State should retain an explicit status snapshot");
    assert_eq!(snapshot.mode, ParticipantStatusSnapshotMode::Compact);
    assert_eq!(snapshot.participants.len(), PARTICIPANTS);
    assert!(snapshot.participants.values().all(|view| {
        view.player_connection.is_some()
            && view.phase.is_some()
            && view.report_age_ms.is_some()
            && view.sample_age_ms.is_none()
            && view.position_sample_age_ms.is_none()
            && view.position_seconds.is_none()
            && view.buffered_ahead_seconds.is_none()
    }));
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .mode,
        ParticipantStatusSnapshotMode::Compact,
        "the room/timestamp cache should remember that the full form overflowed"
    );
    let second = runtime
        .periodic_state_sync_message_for_client_at("client-001", 0.0, true, None, 51.0)
        .expect("later room recipients should reuse the bounded representation");
    let ProtocolMessage::State(second) = second else {
        panic!("periodic status must be State");
    };
    assert_eq!(
        second
            .state
            .participant_status_v1()
            .unwrap()
            .and_then(|extension| extension.snapshot)
            .unwrap()
            .mode,
        ParticipantStatusSnapshotMode::Compact
    );

    const OVERSIZED_COMPACT_PARTICIPANTS: usize = 600;
    for index in PARTICIPANTS..OVERSIZED_COMPACT_PARTICIPANTS {
        let client_id = format!("client-{index:03}");
        let username = format!("participant{index:03}");
        runtime
            .handle_line(
                &client_id,
                &hello(&username, "room", true, false).replace(
                    "\"features\":{",
                    "\"features\":{\"sorotteLargeProtocolFramesV1\":true,",
                ),
            )
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, &client_id);
        let scope = runtime.room_participant_status_scopes["room"];
        send_status(
            &mut runtime,
            &client_id,
            ParticipantStatusStateExtension::new().with_report(
                status_report(1, ParticipantPlaybackPhase::Playing)
                    .with_playback_scope(scope.to_wire(None))
                    .with_timeline_kind(ParticipantTimelineKind::Vod)
                    .with_position_seconds(index as f64)
                    .with_logical_paused(false)
                    .with_playback_rate(1.0)
                    .with_paused_for_cache(false)
                    .with_cache_percent(50.0)
                    .with_buffered_ahead_seconds(8.0)
                    .with_sample_age_ms(0),
            ),
        );
    }
    let unavailable = runtime
        .periodic_state_sync_message_for_client_at("client-000", 0.0, true, None, 52.0)
        .expect("an oversized compact room should retain a bounded explicit projection");
    let encoded = encode_message_line(&unavailable).unwrap();
    assert!(encoded.len() <= sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    let ProtocolMessage::State(unavailable) = unavailable else {
        panic!("periodic status must be State");
    };
    let unavailable = unavailable
        .state
        .participant_status_v1()
        .unwrap()
        .and_then(|extension| extension.snapshot)
        .unwrap();
    assert_eq!(unavailable.mode, ParticipantStatusSnapshotMode::Unavailable);
    assert!(unavailable.participants.is_empty());
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .mode,
        ParticipantStatusSnapshotMode::Unavailable
    );
}

#[test]
fn cached_snapshot_representation_only_escalates_within_one_revision() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    let full = periodic_snapshot(&mut runtime, "alice", 100.0).unwrap();
    assert_eq!(full.mode, ParticipantStatusSnapshotMode::Full);

    let mut different_same_revision_full = full.clone();
    different_same_revision_full.participants.clear();
    runtime.cache_participant_status_snapshot_representation_for_test(
        "alice",
        &different_same_revision_full,
    );
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .participants,
        full.participants,
        "a full representation must not replace another full representation"
    );

    let mismatched_compact = ParticipantStatusSnapshot::new(full.revision + 1, BTreeMap::new())
        .with_mode(ParticipantStatusSnapshotMode::Compact);
    runtime.cache_participant_status_snapshot_representation_for_test("alice", &mismatched_compact);
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .revision,
        full.revision,
        "a degraded representation from another revision must not replace the cache"
    );
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .mode,
        ParticipantStatusSnapshotMode::Full
    );

    let compact = ParticipantStatusSnapshot::new(full.revision, BTreeMap::new())
        .with_mode(ParticipantStatusSnapshotMode::Compact);
    runtime.cache_participant_status_snapshot_representation_for_test("alice", &compact);
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .mode,
        ParticipantStatusSnapshotMode::Compact,
        "same-revision degradation must be shared by later recipients"
    );

    let non_escalating_full = ParticipantStatusSnapshot::new(full.revision, BTreeMap::new());
    runtime
        .cache_participant_status_snapshot_representation_for_test("alice", &non_escalating_full);
    assert_eq!(
        runtime.participant_status_snapshot_cache["room"]
            .snapshot
            .mode,
        ParticipantStatusSnapshotMode::Compact,
        "a same-revision cache representation must never become less bounded"
    );
}

#[test]
fn same_tick_timeout_rebuilds_cached_snapshot_for_remaining_room_members() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "bob"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
        send_status(
            &mut runtime,
            client_id,
            ParticipantStatusStateExtension::new()
                .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
        );
        runtime
            .client_next_periodic_state_at
            .insert(client_id.to_owned(), 200.0);
    }
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 0.0);
    runtime
        .client_last_state_update_at
        .insert("bob".to_owned(), 200.0);
    runtime.set_time_now_override_seconds(Some(200.0));

    let dispatch = runtime
        .collect_dispatch_at(200.0)
        .expect("same-timestamp periodic batch should succeed");
    assert!(runtime.session("alice").is_none());
    let bob_snapshot = decode_directed_lines(&dispatch.outbound_lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == "bob")
        .find_map(|(_, message)| {
            let ProtocolMessage::State(message) = message else {
                return None;
            };
            message
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.snapshot)
        })
        .expect("remaining peer should receive a current-room status snapshot");
    assert_eq!(
        bob_snapshot.participants.keys().collect::<Vec<_>>(),
        ["bob"],
        "a cached snapshot created before timeout must not resurrect the removed participant"
    );
}

#[test]
fn same_tick_timeout_removes_later_sorted_member_before_any_snapshot() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for client_id in ["alice", "zombie"] {
        runtime
            .handle_line(client_id, &hello(client_id, "room", true, false))
            .unwrap();
        acknowledge_current_server_counter(&mut runtime, client_id);
        send_status(
            &mut runtime,
            client_id,
            ParticipantStatusStateExtension::new()
                .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
        );
        runtime
            .client_next_periodic_state_at
            .insert(client_id.to_owned(), 200.0);
    }
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 200.0);
    runtime
        .client_last_state_update_at
        .insert("zombie".to_owned(), 0.0);
    runtime.set_time_now_override_seconds(Some(200.0));

    let dispatch = runtime
        .collect_dispatch_at(200.0)
        .expect("same-timestamp periodic batch should succeed");
    assert!(runtime.session("zombie").is_none());
    let alice_snapshot = decode_directed_lines(&dispatch.outbound_lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == "alice")
        .find_map(|(_, message)| {
            let ProtocolMessage::State(message) = message else {
                return None;
            };
            message
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.snapshot)
        })
        .expect("live peer should receive a current-room status snapshot");
    assert_eq!(
        alice_snapshot.participants.keys().collect::<Vec<_>>(),
        ["alice"],
        "a later-sorted timeout must be removed before the first snapshot is generated"
    );
}

#[test]
fn periodic_timeout_prepass_skips_orphans_and_preserves_fenced_sessions() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("fenced", &hello("fenced", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "fenced");
    runtime
        .client_next_periodic_state_at
        .insert("fenced".to_owned(), 200.0);
    runtime
        .client_last_state_update_at
        .insert("fenced".to_owned(), 0.0);
    runtime
        .playback_barrier_fenced_clients
        .insert("fenced".to_owned());

    runtime
        .client_next_periodic_state_at
        .insert("orphan".to_owned(), 200.0);
    runtime
        .client_last_state_update_at
        .insert("orphan".to_owned(), 0.0);
    runtime.set_time_now_override_seconds(Some(200.0));

    let dispatch = runtime
        .collect_dispatch_at(200.0)
        .expect("incomplete maintenance state should fail closed without aborting the batch");
    assert!(
        runtime.session("fenced").is_some(),
        "a fenced transport remains owned until its disconnect callback"
    );
    assert!(dispatch.transport_actions.iter().any(|action| {
        action.client_id == "fenced" && action.action == ServerTransportAction::Close
    }));
    assert!(
        dispatch
            .transport_actions
            .iter()
            .all(|action| action.client_id != "orphan"),
        "orphaned scheduler state must not synthesize transport actions"
    );
}

#[test]
fn disconnect_clears_cached_status_from_a_retained_empty_room() {
    let mut runtime = ServerRuntime::default();
    runtime.set_permanent_rooms(["room"]);
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    periodic_snapshot(&mut runtime, "alice", 101.0)
        .expect("occupied retained room should build a cached snapshot");
    assert!(
        runtime
            .participant_status_snapshot_cache
            .contains_key("room")
    );

    runtime
        .handle_transport_disconnect_fanout("alice")
        .expect("disconnect should cleanly retain the permanent room");

    assert!(runtime.sessions.is_empty());
    assert!(runtime.client_participant_status.is_empty());
    assert!(runtime.room_participant_status_scopes.contains_key("room"));
    assert!(
        !runtime
            .participant_status_snapshot_cache
            .contains_key("room"),
        "retaining room configuration must not retain departed participant telemetry"
    );
}

#[test]
fn clearing_status_invalidates_distinct_retained_and_session_room_caches() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room-a", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    periodic_snapshot(&mut runtime, "alice", 100.0).unwrap();

    runtime.sessions.get_mut("alice").unwrap().room = "room-b".to_owned();
    periodic_snapshot(&mut runtime, "alice", 100.1).unwrap();
    assert_eq!(runtime.client_participant_status["alice"].room, "room-a");
    assert!(
        runtime
            .participant_status_snapshot_cache
            .contains_key("room-a")
    );
    assert!(
        runtime
            .participant_status_snapshot_cache
            .contains_key("room-b")
    );

    runtime.clear_participant_status_for_client("alice");
    assert!(!runtime.client_participant_status.contains_key("alice"));
    assert!(
        !runtime
            .participant_status_snapshot_cache
            .contains_key("room-a")
    );
    assert!(
        !runtime
            .participant_status_snapshot_cache
            .contains_key("room-b"),
        "the current membership cache must also clear when retained status names an older room"
    );
}

#[test]
fn buffering_forced_state_publishes_final_scope_for_an_immediate_exact_echo() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        let hello = json!({
            "Hello": {
                "username": username,
                "room": { "name": room },
                "version": "1.7.5",
                "features": {
                    SOROTTE_PARTICIPANT_STATUS_V1: true,
                    "sorottePlaybackBarrierV1": true,
                    "sorotteReadinessV2": true,
                },
            },
        });
        runtime.handle_line(client_id, &hello.to_string()).unwrap();
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .unwrap();
    runtime.room_playback_states.insert(
        room.clone(),
        RoomPlaybackState {
            position: 5.0,
            paused: false,
            set_by: Some("alice".to_owned()),
            updated_at_seconds: 100.0,
        },
    );
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":1,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("controller should configure coordinated buffering");

    let paused = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":1,"buffering":true}}}}"#,
        )
        .expect("buffering report should pause the room");
    assert!(runtime.room_playback_state(&room).paused);
    let forced_scope = decode_directed_lines(&paused)
        .into_iter()
        .filter(|(client_id, _)| client_id == "bob-client")
        .find_map(|(_, message)| {
            let ProtocolMessage::State(message) = message else {
                return None;
            };
            message
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.scope)
        })
        .expect("forced buffering State should publish participant scope");
    let current_scope = runtime.room_participant_status_scopes[&room].to_wire(None);
    assert_eq!(
        forced_scope, current_scope,
        "forced State must be encoded after every authority mutation"
    );

    send_status(
        &mut runtime,
        "bob-client",
        ParticipantStatusStateExtension::new().with_report(
            status_report(1, ParticipantPlaybackPhase::Rebuffering)
                .with_playback_scope(forced_scope)
                .with_paused_for_cache(true),
        ),
    );
    acknowledge_current_server_counter(&mut runtime, "alice-client");
    let snapshot = periodic_snapshot(&mut runtime, "alice-client", 100.1)
        .expect("peer should receive the immediate status echo");
    assert_eq!(
        snapshot.participants["bob"].correlation,
        Some(ParticipantStatusCorrelation::Exact)
    );
}

#[test]
fn malformed_participant_status_fallback_never_retains_attacker_tokens() {
    const CANARY: &str = "participant-status-secret-canary-947adc";
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    runtime
        .handle_line(
            "alice",
            &json!({
                "State": {
                    SOROTTE_PARTICIPANT_STATUS_V1: {
                        "report": CANARY,
                    },
                },
            })
            .to_string(),
        )
        .expect("malformed additive status should use compatibility fallback");

    let fallbacks = runtime.drain_compatibility_fallbacks();
    assert!(fallbacks.iter().any(|fallback| matches!(
        fallback,
        ServerCompatibilityFallback::IgnoredInvalidFeatures { context }
            if context == "State.sorotteParticipantStatusV1"
    )));
    assert!(
        !format!("{fallbacks:?}").contains(CANARY),
        "fallback diagnostics must not reproduce malformed attacker-controlled values"
    );
}

#[test]
fn missed_periodic_slots_materialize_only_the_newest_state() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");

    let now = 1_000_000_000.5;
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), now);
    let dispatch = runtime.collect_dispatch_at(now).unwrap();
    let periodic = dispatch
        .outbound_lines
        .iter()
        .filter(|line| line.client_id == "alice")
        .filter(|line| line.delivery == ServerOutboundDelivery::CoalesciblePeriodicState)
        .count();
    assert_eq!(
        periodic, 1,
        "a huge clock jump must materialize only the newest coalescible tick"
    );
    assert_eq!(
        runtime.client_next_periodic_state_at["alice"],
        now + SERVER_STATE_INTERVAL_SECONDS
    );
}

#[test]
fn periodic_scheduler_preserves_elapsed_deadlines_across_wall_clock_rollback() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");

    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 101.0);
    let first = runtime.collect_dispatch_at(101.0).unwrap();
    assert_eq!(
        first
            .outbound_lines
            .iter()
            .filter(|line| {
                line.client_id == "alice"
                    && line.delivery == ServerOutboundDelivery::CoalesciblePeriodicState
            })
            .count(),
        1
    );
    assert_eq!(runtime.client_next_periodic_state_at["alice"], 102.0);

    runtime
        .client_next_periodic_state_at
        .insert("alice".to_owned(), 105.0);
    let same_time = runtime.collect_dispatch_at(101.0).unwrap();
    assert!(same_time.outbound_lines.is_empty());
    assert_eq!(
        runtime.client_next_periodic_state_at["alice"], 105.0,
        "observing the same timestamp twice is not a rollback and must not move a future deadline"
    );

    let rollback = runtime.collect_dispatch_at(90.0).unwrap();
    assert!(rollback.outbound_lines.is_empty());
    assert_eq!(
        runtime.client_next_periodic_state_at["alice"], 105.0,
        "wall rollback must not change an existing elapsed deadline"
    );

    runtime.set_clock_overrides_seconds(Some(91.0), Some(105.0));
    let resumed = runtime.collect_dispatch_at(91.0).unwrap();
    assert_eq!(
        resumed
            .outbound_lines
            .iter()
            .filter(|line| {
                line.client_id == "alice"
                    && line.delivery == ServerOutboundDelivery::CoalesciblePeriodicState
            })
            .count(),
        1,
        "the next monotonic tick after rollback must still publish the one-second heartbeat"
    );
}

#[test]
fn fenced_periodic_ticks_still_advance_retained_report_age_monotonically() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    runtime
        .client_next_periodic_state_at
        .insert("alice".to_owned(), 105.0);
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 105.0);
    runtime.next_server_ignoring_counter("alice");

    let fenced_forward = runtime.collect_dispatch_at(105.0).unwrap();
    assert!(fenced_forward.outbound_lines.is_empty());
    let rollback = runtime.collect_dispatch_at(102.0).unwrap();
    assert!(rollback.outbound_lines.is_empty());

    acknowledge_current_server_counter(&mut runtime, "alice");
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 103.0);
    runtime.set_clock_overrides_seconds(Some(103.0), Some(106.0));
    let resumed = runtime.collect_dispatch_at(103.0).unwrap();
    let snapshot = decode_directed_lines(&resumed.outbound_lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == "alice")
        .find_map(|(_, message)| {
            let ProtocolMessage::State(message) = message else {
                return None;
            };
            message
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.snapshot)
        })
        .expect("the unfenced due tick should publish a complete snapshot");
    let alice = &snapshot.participants["alice"];
    assert_eq!(alice.availability, ParticipantStatusAvailability::Delayed);
    assert!(
        alice.report_age_ms.is_some_and(|age| age >= 5_000),
        "time observed while projection was fenced must remain in the report age",
    );
}

#[test]
fn periodic_state_preserves_ping_metadata_and_honors_the_ignore_fence() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    fixture_issued_ping_echo(&mut runtime, "alice", 90.0, 2.0);
    runtime.queue_client_latency_calculation("alice", 124.1);
    runtime.queue_client_ignoring_counter("alice", 7);

    let message = runtime
        .periodic_state_sync_message_for_client_at("alice", 5.0, false, Some("alice"), 100.25)
        .expect("an acknowledged client should receive its periodic State");
    let ProtocolMessage::State(message) = message else {
        panic!("periodic sync should be State");
    };
    assert_eq!(
        message
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.set_by.as_deref()),
        Some("alice"),
        "participant-status periodic State must preserve canonical playstate authority"
    );
    let ping = message
        .state
        .ping
        .expect("periodic sync should preserve ping metadata");
    assert_eq!(ping.latency_calculation, Some(100.25));
    assert_eq!(ping.client_latency_calculation, Some(124.35));
    assert_eq!(ping.server_rtt, Some(10.0));
    assert_eq!(
        message
            .state
            .ignoring_on_the_fly
            .and_then(|ignoring| ignoring.client),
        Some(7)
    );

    runtime.next_server_ignoring_counter("alice");
    assert!(
        runtime
            .periodic_state_sync_message_for_client_at("alice", 6.0, false, None, 101.0)
            .is_none(),
        "unacknowledged reliable State must fence periodic coalescible State"
    );
}

#[test]
fn periodic_status_is_split_from_reliable_client_passthrough() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice", &hello("alice", "room", true, false))
        .unwrap();
    acknowledge_current_server_counter(&mut runtime, "alice");
    send_status(
        &mut runtime,
        "alice",
        ParticipantStatusStateExtension::new()
            .with_report(status_report(1, ParticipantPlaybackPhase::Playing)),
    );
    runtime.queue_client_ignoring_counter("alice", 7);
    runtime
        .client_last_state_update_at
        .insert("alice".to_owned(), 101.0);
    runtime
        .client_next_periodic_state_at
        .insert("alice".to_owned(), 101.0);

    let dispatch = runtime.collect_dispatch_at(101.0).unwrap();
    let reliable = dispatch
        .outbound_lines
        .iter()
        .find(|line| line.client_id == "alice" && line.delivery == ServerOutboundDelivery::Reliable)
        .expect("client passthrough metadata should retain its reliable delivery");
    let ProtocolMessage::State(reliable_state) = decode_message_line(&reliable.line).unwrap()
    else {
        panic!("reliable passthrough should be a State message");
    };
    assert_eq!(
        reliable_state
            .state
            .ignoring_on_the_fly
            .as_ref()
            .and_then(|value| value.client),
        Some(7),
    );
    assert!(
        reliable_state
            .state
            .participant_status_v1()
            .unwrap()
            .is_none(),
        "population-sized advisory snapshots must never ride a reliable frame",
    );

    let periodic = dispatch
        .outbound_lines
        .iter()
        .find(|line| {
            line.client_id == "alice"
                && line.delivery == ServerOutboundDelivery::CoalesciblePeriodicState
        })
        .expect("status should be emitted separately on the coalescible lane");
    let ProtocolMessage::State(periodic_state) = decode_message_line(&periodic.line).unwrap()
    else {
        panic!("coalescible participant status should be a State message");
    };
    assert!(periodic_state.state.ignoring_on_the_fly.is_none());
    assert!(
        periodic_state
            .state
            .participant_status_v1()
            .unwrap()
            .and_then(|extension| extension.snapshot)
            .is_some(),
    );
}

#[test]
fn protocol_line_limit_accepts_the_boundary_and_rejects_the_next_byte() {
    assert!(!crate::runtime_maintenance::protocol_line_exceeds_maximum(
        sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES
    ));
    assert!(crate::runtime_maintenance::protocol_line_exceeds_maximum(
        sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES + 1
    ));
}
