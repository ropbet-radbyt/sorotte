use super::ordered_events::{OrderedLoadInstall, snapshot_known_copy};
use super::*;
use sorotte_player_api::{
    DisconnectedPlayer, PlayerAdapter, PlayerCapabilities, PlayerCapability, PlayerCommand,
    PlayerCommandId, PlayerError, PlayerMediaGeneration, PlayerObservationTimestamp,
    PlayerPhysicalLoadOutcome, PlayerTransportPhase, PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    CommitStartPayload, ParticipantPlaybackPhase, ParticipantPlayerConnection,
    PlaybackBarrierParticipantStatus, PlaybackBarrierPhase, PlaybackBarrierStatusPayload,
    ProtocolMessage, RoomBufferingPhase, RoomBufferingStatusPayload, SetPayload,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

mod seek_echo_tests;

#[test]
fn playback_barrier_start_defaults_to_immediate() {
    assert_eq!(PlaybackBarrierStartConfig::default().policy, None);
}

#[test]
fn changed_media_transport_refresh_does_not_start_a_room_barrier() {
    let mut session = barrier_session();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("controller authority should apply");
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });

    let plan = runtime.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("joined-room-episode").unwrap(),
        MediaTransportKind::LocalFile,
        1.0,
    );

    assert_eq!(plan.load_intent, MediaLoadIntent::TransportRefresh);
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_none(),
        "room participation must not queue a controller-owned start barrier"
    );
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "room participation must not emit a playback-barrier request"
    );
}

fn participant_status_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .expect("participant-status Hello should apply");
    session
}

fn reports_in(messages: Vec<ProtocolMessage>) -> Vec<ParticipantStatusReport> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state
                .state
                .participant_status_v1()
                .ok()
                .flatten()
                .and_then(|extension| extension.report),
            _ => None,
        })
        .collect()
}

fn participant_status_accepted_barrier_fixture() -> (RuntimePlaybackCoordination, ClientSession) {
    let mut coordination = RuntimePlaybackCoordination::default();
    let local_media_generation = coordination
        .prepare_media(
            LogicalMediaId::new("participant-status-accepted-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordination.barrier.accepted_barrier = Some(PlaybackBarrierOperation {
        local_media_generation,
        load_intent: MediaLoadIntent::NewPlayback,
        include_start_barrier: true,
        request_id: "participant-status-request".to_owned(),
        request_nonce: 7,
        logical_media_id: "participant-status-accepted-media".to_owned(),
        room: "room1".to_owned(),
    });
    (coordination, barrier_session())
}

fn participant_status_transport_fixture() -> (RuntimePlaybackCoordination, u64) {
    let mut coordination = RuntimePlaybackCoordination::default();
    let media_generation = coordination
        .prepare_media(
            LogicalMediaId::new("participant-status-transport-fixture").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    (coordination, media_generation)
}

#[test]
fn participant_status_barrier_revision_sources_are_generation_exact() {
    let mut commit_session = barrier_session();
    apply_barrier_extension(
        &mut commit_session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    12,
                    "revision-source-commit",
                    0.0,
                    PlaybackBarrierPolicy::Controller,
                )
                .with_request_nonce(1),
            )
            .with_commit(CommitStartPayload::new(12, 102, 0.0, 0.0, 10.0))
            .with_status(barrier_status(
                12,
                Some(102),
                PlaybackBarrierPhase::Committed,
            )),
    );
    assert_eq!(
        RuntimePlaybackCoordination::participant_status_state_revision_for_generation(
            &commit_session,
            12,
        ),
        Some(102),
    );

    let mut policy_session = barrier_session();
    apply_barrier_extension(
        &mut policy_session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(
            RoomBufferingPolicyPayload::new(13, RoomBufferingPolicy::PauseAnyEligible)
                .with_state_revision(103),
        ),
    );
    assert_eq!(
        RuntimePlaybackCoordination::participant_status_state_revision_for_generation(
            &policy_session,
            13,
        ),
        Some(103),
    );
}

#[test]
fn participant_status_accepted_generation_requires_every_operation_identity_axis() {
    let evaluate_prepare = |request_id: &str, request_nonce: u64, logical_media_id: &str| {
        let (coordination, mut session) = participant_status_accepted_barrier_fixture();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new().with_prepare(
                PrepareMediaPayload::new(
                    41,
                    logical_media_id,
                    0.0,
                    PlaybackBarrierPolicy::Controller,
                )
                .with_request_id(request_id)
                .with_request_nonce(request_nonce),
            ),
        );
        coordination.accepted_participant_status_media_generation(&session)
    };
    assert_eq!(
        evaluate_prepare(
            "participant-status-request",
            7,
            "participant-status-accepted-media",
        ),
        Some(41),
    );
    assert_eq!(
        evaluate_prepare("wrong-request", 7, "participant-status-accepted-media",),
        None,
    );
    assert_eq!(
        evaluate_prepare(
            "participant-status-request",
            8,
            "participant-status-accepted-media",
        ),
        None,
    );
    assert_eq!(
        evaluate_prepare("participant-status-request", 7, "wrong-media"),
        None,
    );

    let evaluate_policy = |request_id: &str, request_nonce: u64| {
        let (coordination, mut session) = participant_status_accepted_barrier_fixture();
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new().with_buffering_policy(
                RoomBufferingPolicyPayload::new(42, RoomBufferingPolicy::PauseAnyEligible)
                    .with_request_id(request_id)
                    .with_request_nonce(request_nonce),
            ),
        );
        coordination.accepted_participant_status_media_generation(&session)
    };
    assert_eq!(evaluate_policy("participant-status-request", 7), Some(42),);
    assert_eq!(evaluate_policy("wrong-request", 7), None);
    assert_eq!(evaluate_policy("participant-status-request", 8), None);

    let (mut wrong_generation, mut valid_session) = participant_status_accepted_barrier_fixture();
    apply_barrier_extension(
        &mut valid_session,
        PlaybackBarrierSetExtension::new().with_prepare(
            PrepareMediaPayload::new(
                41,
                "participant-status-accepted-media",
                0.0,
                PlaybackBarrierPolicy::Controller,
            )
            .with_request_id("participant-status-request")
            .with_request_nonce(7),
        ),
    );
    wrong_generation
        .barrier
        .accepted_barrier
        .as_mut()
        .unwrap()
        .local_media_generation += 1;
    assert_eq!(
        wrong_generation.accepted_participant_status_media_generation(&valid_session),
        None,
    );
    let (mut wrong_room, mut valid_session) = participant_status_accepted_barrier_fixture();
    apply_barrier_extension(
        &mut valid_session,
        PlaybackBarrierSetExtension::new().with_prepare(
            PrepareMediaPayload::new(
                41,
                "participant-status-accepted-media",
                0.0,
                PlaybackBarrierPolicy::Controller,
            )
            .with_request_id("participant-status-request")
            .with_request_nonce(7),
        ),
    );
    wrong_room.barrier.accepted_barrier.as_mut().unwrap().room = "other-room".to_owned();
    assert_eq!(
        wrong_room.accepted_participant_status_media_generation(&valid_session),
        None,
    );
}

#[test]
fn participant_status_starting_seek_detection_accepts_each_independent_signal() {
    let (mut phase_signal, _) = participant_status_transport_fixture();
    let mut phase_update = transport(1, 1.0, PlayerTransportPhase::Seeking, 5.0);
    phase_update.seeking = Some(false);
    phase_signal.observe_transport(phase_update, 1.0);
    assert_eq!(
        phase_signal.participant_status_phase(),
        ParticipantPlaybackPhase::Seeking,
    );

    let (mut boolean_signal, _) = participant_status_transport_fixture();
    let mut boolean_update = transport(1, 1.0, PlayerTransportPhase::Playing, 5.0);
    boolean_update.core_idle = Some(true);
    boolean_update.seeking = Some(true);
    boolean_signal.observe_transport(boolean_update, 1.0);
    assert_eq!(
        boolean_signal.participant_status_phase(),
        ParticipantPlaybackPhase::Seeking,
    );
}

#[test]
fn participant_status_lifecycle_wait_clock_rejects_each_invalid_axis() {
    for (waiting_since, now_seconds) in [(1.0, f64::NAN), (f64::NAN, 1.0), (2.0, 1.0)] {
        let mut coordination = RuntimePlaybackCoordination {
            transport_telemetry_wait_started_at_seconds: Some(waiting_since),
            ..RuntimePlaybackCoordination::default()
        };
        assert!(!coordination.participant_status_telemetry_wait_is_current(now_seconds));
        assert!(coordination.participant_status_owner_clock_invalidated);
        assert!(
            coordination
                .transport_telemetry_wait_started_at_seconds
                .is_none()
        );
    }
    let mut equal_timestamp = RuntimePlaybackCoordination {
        transport_telemetry_wait_started_at_seconds: Some(1.0),
        ..RuntimePlaybackCoordination::default()
    };
    assert!(equal_timestamp.participant_status_telemetry_wait_is_current(1.0));
}

#[test]
fn participant_status_transport_commit_guards_are_independent() {
    let commit = |mut coordination: RuntimePlaybackCoordination,
                  observation: PlayerTransportObservation| {
        let position_update = observation.clone();
        coordination.commit_mapped_transport_observation(
            observation,
            &position_update,
            2.0,
            2.0,
            None,
            false,
            false,
        )
    };

    let (mut fenced, generation) = participant_status_transport_fixture();
    fenced.transport_telemetry_lifecycle_fence_at_seconds = Some(2.0);
    assert_eq!(
        commit(fenced, PlayerTransportObservation::new(generation, 1.0)),
        MappedTransportCommitOutcome::LifecycleFenced
    );

    let (wrong_generation, generation) = participant_status_transport_fixture();
    assert_eq!(
        commit(
            wrong_generation,
            PlayerTransportObservation::new(generation + 1, 1.0),
        ),
        MappedTransportCommitOutcome::Rejected
    );

    let (mut stale, generation) = participant_status_transport_fixture();
    stale.latest_observation = Some(PlayerTransportObservation::new(generation, 2.0));
    assert_eq!(
        commit(stale, PlayerTransportObservation::new(generation, 1.0)),
        MappedTransportCommitOutcome::Rejected
    );

    for (external_now, delivery_reference, offset) in [
        (f64::NAN, 2.0, None),
        (2.0, f64::NAN, None),
        (2.0, 2.0, Some(f64::NAN)),
    ] {
        let (mut coordination, generation) = participant_status_transport_fixture();
        let observation = PlayerTransportObservation::new(generation, 1.0);
        assert_eq!(
            coordination.commit_mapped_transport_observation(
                observation.clone(),
                &observation,
                external_now,
                delivery_reference,
                offset,
                false,
                false,
            ),
            MappedTransportCommitOutcome::Rejected
        );
    }
}

#[test]
fn participant_status_transport_lifecycle_fence_accepts_its_exact_boundary() {
    let (mut older, _) = participant_status_transport_fixture();
    older.transport_telemetry_lifecycle_fence_at_seconds = Some(2.0);
    assert!(
        older
            .observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 7.0), 1.0,)
            .is_empty()
    );
    assert!(!older.transport_telemetry_observed);
    assert!(older.latest_observation.is_none());

    let (mut at_boundary, generation) = participant_status_transport_fixture();
    at_boundary.transport_telemetry_lifecycle_fence_at_seconds = Some(2.0);
    at_boundary.observe_transport(transport(1, 2.0, PlayerTransportPhase::Playing, 7.0), 2.0);
    let accepted = at_boundary
        .latest_observation
        .expect("an observation exactly on the lifecycle fence must be accepted");
    assert_eq!(accepted.media_generation, generation);
    assert_eq!(accepted.observed_at_seconds, 2.0);
    assert_eq!(accepted.position_seconds, Some(7.0));
    assert!(at_boundary.transport_telemetry_observed);
}

#[test]
fn participant_status_transport_clock_reset_truth_table_is_one_way() {
    let commit_sparse = |last_external_now: f64,
                         external_now: f64,
                         owner_invalidated: bool,
                         last_received_at: Option<f64>| {
        let (mut coordination, generation) = participant_status_transport_fixture();
        coordination.latest_observation = Some(
            PlayerTransportObservation::new(generation, 1.0)
                .with_position(9.0)
                .with_logical_pause(false),
        );
        coordination.last_external_now_seconds = Some(last_external_now);
        coordination.participant_status_owner_clock_invalidated = owner_invalidated;
        coordination.last_transport_telemetry_received_at_seconds = last_received_at;
        let sparse = PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Playing);
        assert_eq!(
            coordination.commit_mapped_transport_observation(
                sparse.clone(),
                &sparse,
                external_now,
                external_now,
                None,
                false,
                false,
            ),
            MappedTransportCommitOutcome::Committed
        );
        coordination
            .latest_observation
            .as_ref()
            .and_then(|observation| observation.position_seconds)
    };

    assert_eq!(commit_sparse(10.0, 10.0, false, None), Some(9.0));
    assert_eq!(commit_sparse(10.0, 9.0, false, None), None);
    assert_eq!(commit_sparse(10.0, 11.0, true, None), None);
    assert_eq!(commit_sparse(f64::NAN, 11.0, false, None), None);
    assert_eq!(
        commit_sparse(
            0.0,
            PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS,
            false,
            Some(0.0),
        ),
        Some(9.0),
        "the exact stale threshold is still current; only a larger gap starts a new epoch",
    );
}

#[test]
fn participant_status_room_scope_is_invalidated_by_each_local_context_axis() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    let local_media_generation = coordination
        .prepare_media(
            LogicalMediaId::new("participant-status-local-scope").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;

    coordination
        .participant_status
        .participant_status_room_scope = Some(ParticipantStatusRoomScope {
        room: "room1".to_owned(),
        local_media_generation: local_media_generation + 1,
        media_generation: 41,
        state_revision: Some(3),
        transport_revision: Some(5),
    });
    coordination.refresh_participant_status_room_scope(&session);
    assert!(
        coordination
            .participant_status
            .participant_status_room_scope
            .is_none(),
        "a new local media generation must retire the prior room scope"
    );

    coordination
        .participant_status
        .participant_status_room_scope = Some(ParticipantStatusRoomScope {
        room: "room2".to_owned(),
        local_media_generation,
        media_generation: 41,
        state_revision: Some(3),
        transport_revision: Some(5),
    });
    coordination.refresh_participant_status_room_scope(&session);
    assert!(
        coordination
            .participant_status
            .participant_status_room_scope
            .is_none(),
        "a room change must retire the prior room scope even for the same local media"
    );
}

#[test]
fn participant_status_adopted_generation_requires_matching_logical_media() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .unwrap();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    41,
                    "authoritative-logical-media",
                    0.0,
                    PlaybackBarrierPolicy::Controller,
                )
                .with_request_id("participant-status-generation-only-correlation")
                .with_request_nonce(7),
            )
            .with_status(barrier_status(41, None, PlaybackBarrierPhase::Preparing)),
    );

    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("different-local-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.desired_fingerprint = Some(RoomDesiredFingerprint {
        paused: false,
        position_seconds: 0.0,
        do_seek: false,
        local_echo: false,
        barrier_media_generation: Some(41),
        barrier_state_revision: None,
        buffering_media_generation: None,
        buffering_state_revision: None,
    });

    let prepare = session
        .playback_barrier_prepare()
        .expect("the adopted generation must have a current prepare");
    assert_eq!(prepare.media_generation, 41);
    assert!(
        !coordination.current_logical_media_matches(&prepare.logical_media_id),
        "the regression requires generation-only correlation"
    );

    coordination.refresh_participant_status_room_scope(&session);
    assert!(
        coordination
            .participant_status
            .participant_status_room_scope
            .is_none(),
        "matching only the generation must not bind unrelated local media to room scope"
    );
}

#[test]
fn participant_status_report_requires_negotiated_server_capability() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":false}}}"#,
        )
        .unwrap();
    assert!(session.is_active());
    assert_eq!(session.room(), Some("room1"));
    assert!(!session.server_participant_status_v1_supported());

    assert!(
        RuntimePlaybackCoordination::default()
            .pending_participant_status_report(&session, true, 1.0)
            .is_none(),
        "an otherwise active room session must not report without negotiated support"
    );
}

#[test]
fn participant_status_heartbeat_is_silent_without_negotiated_capability() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":false}}}"#,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    assert!(!runtime.run_participant_status_heartbeat(1.0));
    assert!(
        runtime.flush_queued_protocol_messages().is_empty(),
        "a legacy peer must not receive an empty advisory State"
    );
}

#[test]
fn participant_status_heartbeat_queues_only_the_negotiated_extension() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    assert!(runtime.run_participant_status_heartbeat(1.0));
    let messages = runtime.flush_queued_protocol_messages();
    assert_eq!(messages.len(), 1);
    let ProtocolMessage::State(state) = &messages[0] else {
        panic!("status heartbeat should use State");
    };
    assert!(state.state.playstate.is_none());
    assert!(state.state.ping.is_none());
    assert!(
        state
            .state
            .participant_status_v1()
            .unwrap()
            .and_then(|extension| extension.report)
            .is_some(),
        "the status-only heartbeat must contain the negotiated extension"
    );
}

#[test]
fn participant_status_heartbeat_backlog_stays_bounded_when_chat_interleaves() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorotteParticipantStatusV1":true}}}"#,
        )
        .expect("chat and participant-status capabilities should negotiate");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    for sequence in 1..=128 {
        assert!(runtime.run_participant_status_heartbeat(sequence as f64));
        assert!(
            runtime
                .run_send_chat_message(format!("chat-{sequence}"))
                .expect("chat should queue")
        );
    }

    let messages = runtime.flush_queued_protocol_messages();
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ProtocolMessage::Chat(_)))
            .count(),
        128,
        "reliable chat must remain queued"
    );
    let reports = reports_in(messages);
    assert_eq!(
        reports.len(),
        1,
        "reachable periodic advisory status must remain independently coalescible"
    );
    assert_eq!(reports[0].report_sequence, 128);
}

#[test]
fn participant_status_heartbeat_does_not_treat_equal_time_as_rollback() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination
        .take_participant_status_report(&session, false, 10.0)
        .expect("the initial fingerprint should report");

    assert!(
        coordination
            .pending_participant_status_report(&session, true, 10.0)
            .is_none(),
        "the same timestamp is neither rollback nor a due heartbeat"
    );
}

#[test]
fn participant_status_room_switch_confirmation_requires_authoritative_membership() {
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.begin_participant_status_room_switch("room2", Some("room1"));
    let session_without_authoritative_membership = ClientSession::default();

    coordination
        .confirm_participant_status_room_membership(&session_without_authoritative_membership);
    assert_eq!(
        coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .as_deref(),
        Some("room2"),
        "a pending fence must survive until the session observes authoritative membership"
    );

    let mut partial_membership = participant_status_session();
    partial_membership.model.room.name = Some("room2".to_owned());
    coordination.confirm_participant_status_room_membership(&partial_membership);
    assert_eq!(
        coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .as_deref(),
        Some("room2"),
        "the current-room field alone cannot confirm the target membership"
    );

    partial_membership.model.room.name = Some("room1".to_owned());
    partial_membership
        .model
        .room
        .users
        .get_mut("alice")
        .unwrap()
        .room = Some("room2".to_owned());
    coordination.confirm_participant_status_room_membership(&partial_membership);
    assert_eq!(
        coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .as_deref(),
        Some("room2"),
        "the self membership row alone cannot confirm the target room"
    );

    partial_membership.model.room.name = Some("room2".to_owned());
    coordination.confirm_participant_status_room_membership(&partial_membership);
    assert!(
        coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .is_none(),
        "matching authoritative room and self membership must release the fence"
    );
}

#[test]
fn participant_status_runtime_reports_transport_transitions_and_periodic_heartbeats() {
    let base_now = unix_wall_clock_time_seconds_legacy_compatible();
    let player = CoordinatedTestPlayer {
        advertises_telemetry: true,
        ..CoordinatedTestPlayer::default()
    };
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        player,
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("private-local-identity").unwrap(),
        MediaTransportKind::NetworkVod,
        base_now,
    );
    let mut playing = transport(1, 1.0, PlayerTransportPhase::Playing, 42.5);
    playing.playback_rate = Some(1.0);
    playing.cache_buffering_percent = Some(67.0);
    playing.buffered_ahead_seconds = Some(8.25);
    runtime.player.transport_updates.push_back(playing);

    runtime
        .drain_player_transport_coordination(base_now)
        .expect("playing observation should drain");
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.report_sequence, 1);
    assert_eq!(
        report.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(report.phase, ParticipantPlaybackPhase::Playing);
    assert_eq!(report.position_seconds, Some(42.5));
    assert_eq!(report.logical_paused, Some(false));
    assert_eq!(report.playback_rate, Some(1.0));
    assert_eq!(report.buffered_ahead_seconds, Some(8.25));
    assert_eq!(report.cache_percent, Some(67.0));
    assert_eq!(
        report.playback_scope, None,
        "client-local media generations must not be published as room generations"
    );

    runtime
        .playback_coordination
        .participant_status
        .last_participant_status_sent_at_seconds =
        Some(base_now - PARTICIPANT_STATUS_HEARTBEAT_SECONDS);
    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].report_sequence, 2);
    assert_eq!(reports[0].position_seconds, Some(42.5));

    runtime.player.transport_updates.push_back(transport(
        1,
        2.0,
        PlayerTransportPhase::Playing,
        43.5,
    ));
    runtime
        .drain_player_transport_coordination(base_now + 1.0)
        .expect("same coarse phase should drain");
    assert!(
        reports_in(runtime.flush_queued_protocol_messages()).is_empty(),
        "ordinary position movement waits for the periodic cadence"
    );

    let mut buffering = transport(1, 3.0, PlayerTransportPhase::Rebuffering, 43.5);
    buffering.buffered_ahead_seconds = Some(0.25);
    runtime.player.transport_updates.push_back(buffering);
    runtime
        .drain_player_transport_coordination(base_now + 2.0)
        .expect("rebuffering transition should drain");
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].report_sequence, 3);
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Rebuffering);
    assert_eq!(reports[0].buffered_ahead_seconds, Some(0.25));
}

#[test]
fn failed_participant_status_delivery_retries_the_identical_sequence_transactionally() {
    #[derive(Default)]
    struct RejectFirstParticipantStatusControl {
        attempts: Vec<ParticipantStatusReport>,
    }

    impl ClientEffectSink for RejectFirstParticipantStatusControl {
        fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
            if let ClientEffect::SendState(state) = effect
                && let Some(report) = state
                    .participant_status_v1()
                    .expect("test participant-status payload should decode")
                    .and_then(|extension| extension.report)
            {
                self.attempts.push(report);
                if self.attempts.len() == 1 {
                    return Err(ClientEffectError::OperationFailed(
                        "forced participant-status delivery failure".to_owned(),
                    ));
                }
            }
            Ok(())
        }
    }

    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        RejectFirstParticipantStatusControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("participant-status-transaction").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );

    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
        .expect_err("the first participant-status delivery should be rejected");
    assert_eq!(runtime.control.attempts.len(), 1);
    assert_eq!(runtime.control.attempts[0].report_sequence, 1);
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .next_participant_status_sequence,
        0
    );

    assert!(
        runtime
            .emit_participant_status_transition(0.1)
            .expect("the identical pending report should retry successfully")
    );
    assert_eq!(runtime.control.attempts.len(), 2);
    assert_eq!(runtime.control.attempts[0], runtime.control.attempts[1]);
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .next_participant_status_sequence,
        1
    );

    assert!(
        runtime
            .set_external_player_availability(ExternalPlayerAvailability::Failed, 0.2)
            .expect("the next transition should deliver")
    );
    assert_eq!(runtime.control.attempts.len(), 3);
    assert_eq!(runtime.control.attempts[2].report_sequence, 2);
}

#[test]
fn session_update_retries_failed_participant_status_without_committing_sequence() {
    #[derive(Default)]
    struct RejectFirstSessionUpdateStatusControl {
        attempts: Vec<ParticipantStatusReport>,
    }

    impl ClientEffectSink for RejectFirstSessionUpdateStatusControl {
        fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
            if let ClientEffect::SendState(state) = effect
                && let Some(report) = state
                    .participant_status_v1()
                    .expect("test participant-status payload should decode")
                    .and_then(|extension| extension.report)
            {
                self.attempts.push(report);
                if self.attempts.len() == 1 {
                    return Err(ClientEffectError::OperationFailed(
                        "forced session-update participant-status failure".to_owned(),
                    ));
                }
            }
            Ok(())
        }
    }

    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        RejectFirstSessionUpdateStatusControl::default(),
    );
    let hello = r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#;

    runtime
        .session_mut()
        .apply_message_json_at(hello, 0.0)
        .expect("the Hello should apply despite advisory delivery failure");
    assert_eq!(runtime.control.attempts.len(), 1);
    assert_eq!(runtime.control.attempts[0].report_sequence, 1);
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .next_participant_status_sequence,
        0,
        "a swallowed ClientSessionUpdate sink error must not commit the report"
    );

    runtime
        .session_mut()
        .apply_message_json_at(hello, 0.1)
        .expect("the replacement Hello should retry the pending advisory report");
    assert_eq!(runtime.control.attempts.len(), 2);
    assert_eq!(runtime.control.attempts[0], runtime.control.attempts[1]);
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .next_participant_status_sequence,
        1
    );

    runtime
        .session_mut()
        .apply_message_json_at(hello, 0.2)
        .expect("an unchanged replacement Hello should remain a no-op");
    assert_eq!(
        runtime.control.attempts.len(),
        2,
        "the successful retry must commit its fingerprint exactly once"
    );
}

#[test]
fn sparse_transport_updates_preserve_the_oldest_retained_evidence_age() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("participant-status-sparse-evidence").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut complete = transport(1, 10.0, PlayerTransportPhase::Playing, 42.0);
    complete.playback_rate = Some(1.0);
    complete.cache_buffering_percent = Some(70.0);
    complete.buffered_ahead_seconds = Some(8.0);
    coordination.observe_transport(complete, 10.0);
    coordination
        .take_participant_status_report(&session, true, 10.0)
        .expect("the complete observation should report");

    let sparse_position = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(12.0)),
    )
    .with_position_seconds(44.0);
    coordination.observe_transport(sparse_position, 12.0);
    let report = coordination
        .take_participant_status_report(&session, true, 12.0)
        .expect("the periodic report should retain sparse telemetry fields");

    assert_eq!(report.position_seconds, Some(44.0));
    assert_eq!(
        report.position_sample_age_ms,
        Some(0),
        "a sparse position refresh owns an independent fresh position age"
    );
    assert_eq!(report.logical_paused, Some(false));
    assert_eq!(report.playback_rate, Some(1.0));
    assert_eq!(report.paused_for_cache, Some(false));
    assert_eq!(report.buffered_ahead_seconds, Some(8.0));
    assert_eq!(report.cache_percent, Some(70.0));
    assert_eq!(
        report.sample_age_ms,
        Some(2_000),
        "sample age must describe the oldest field still present in the report"
    );
}

#[test]
fn participant_status_omits_out_of_range_optional_telemetry_before_encoding() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.mark_transport_telemetry_available();
    let mut observation = transport(
        1,
        0.0,
        PlayerTransportPhase::Playing,
        PARTICIPANT_STATUS_MAX_POSITION_SECONDS + 1.0,
    );
    observation.playback_rate = Some(PARTICIPANT_STATUS_MAX_PLAYBACK_RATE + 0.1);
    observation.buffered_ahead_seconds = Some(PARTICIPANT_STATUS_MAX_BUFFERED_AHEAD_SECONDS + 1.0);
    observation.cache_buffering_percent = Some(150.0);
    coordination.observe_transport(observation, 100.0);
    coordination.last_transport_telemetry_received_at_seconds = Some(200.0);

    let report = coordination
        .take_participant_status_report(&session, true, 200.0)
        .expect("invalid optional observations must not suppress the status heartbeat");
    assert_eq!(
        report.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(report.position_seconds, None);
    assert_eq!(report.playback_rate, None);
    assert_eq!(report.buffered_ahead_seconds, None);
    assert_eq!(report.cache_percent, None);
    assert_eq!(
        report.sample_age_ms,
        Some(PARTICIPANT_STATUS_MAX_SAMPLE_AGE_MILLIS)
    );
}

#[test]
fn participant_status_report_sanitizes_seeking_precision_before_encoding() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("participant-status-seeking-sanitizer").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut seeking = transport(1, 1.0, PlayerTransportPhase::Seeking, 42.0);
    seeking.playback_rate = Some(1.25);
    coordination.observe_transport(seeking, 1.0);

    let report = coordination
        .take_participant_status_report(&session, true, 1.0)
        .expect("the coarse seeking transition should remain reportable");
    assert_eq!(
        report.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(report.phase, ParticipantPlaybackPhase::Seeking);
    assert_eq!(report.position_seconds, None);
    assert_eq!(report.logical_paused, None);
    assert_eq!(report.playback_rate, None);
    assert_eq!(report.position_sample_age_ms, None);
}

#[test]
fn public_external_epoch_observation_emits_participant_status_transition_immediately() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("external-participant-status-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
        .expect("external lifecycle status should queue");
    runtime.flush_queued_protocol_messages();
    assert!(
        !runtime
            .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.1,)
            .expect("repeated lifecycle status should be a no-op")
    );
    assert!(runtime.flush_queued_protocol_messages().is_empty());

    let epoch = runtime.playback_transport_adapter_epoch();
    runtime.observe_external_player_transport_at_epoch(
        transport(1, 1.0, PlayerTransportPhase::Playing, 12.5),
        1.0,
        epoch,
    );

    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Playing);
    assert_eq!(reports[0].position_seconds, Some(12.5));

    runtime.observe_external_player_transport(
        transport(1, 2.0, PlayerTransportPhase::Rebuffering, 13.0),
        2.0,
    );
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Rebuffering);

    runtime
        .observe_external_player_end_of_file(2.1)
        .expect("external EOF should be reported");
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Ended);
    assert_eq!(
        runtime.session().local_paused(),
        Some(true),
        "legacy EOF must update the same logical-pause projection as an ordered terminal"
    );
}

#[test]
fn participant_status_public_external_epoch_rebase_emits_transition_immediately() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("external-participant-status-rebase").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
        .expect("external lifecycle status should queue");
    runtime.flush_queued_protocol_messages();

    let epoch = runtime.playback_transport_adapter_epoch();
    runtime.rebase_external_player_transport_at_epoch(
        transport(1, 1.0, PlayerTransportPhase::Playing, 12.5),
        1.0,
        epoch,
    );

    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Playing);
    assert_eq!(reports[0].position_seconds, Some(12.5));
}

#[test]
fn participant_status_legacy_position_fallback_requires_a_legacy_player() {
    assert!(participant_status_legacy_position_fallback(None, false));
    assert!(!participant_status_legacy_position_fallback(None, true));
    assert!(participant_status_legacy_position_fallback(
        Some(ExternalPlayerAvailability::Connecting),
        false,
    ));
    assert!(!participant_status_legacy_position_fallback(
        Some(ExternalPlayerAvailability::Connecting),
        true,
    ));

    for availability in [
        ExternalPlayerAvailability::Unavailable,
        ExternalPlayerAvailability::TelemetryUnavailable,
        ExternalPlayerAvailability::Disconnected,
        ExternalPlayerAvailability::Failed,
    ] {
        for transport_telemetry_ever_observed in [false, true] {
            assert!(
                !participant_status_legacy_position_fallback(
                    Some(availability),
                    transport_telemetry_ever_observed,
                ),
                "an explicit {availability:?} lifecycle must fence the legacy position "
            );
        }
    }
}

#[test]
fn participant_status_player_availability_ages_and_obeys_explicit_lifecycle() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("availability-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    assert!(
        coordination
            .set_external_player_availability(ExternalPlayerAvailability::Unavailable, -1.0)
    );
    assert!(!coordination.transport_telemetry_available);
    assert_eq!(
        coordination.transport_telemetry_wait_started_at_seconds,
        None
    );
    coordination.mark_transport_telemetry_available();
    assert!(
        coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
    );
    assert!(coordination.transport_telemetry_available);
    assert_eq!(
        coordination.transport_telemetry_wait_started_at_seconds,
        Some(0.0)
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 0.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Starting
    );
    assert!(
        !coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 1.0)
    );
    assert_eq!(
        coordination
            .take_participant_status_report(
                &session,
                true,
                PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS + 0.1,
            )
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Unavailable,
        "Connecting without a first sample must age instead of remaining fresh forever"
    );

    let mut playing = transport(1, 6.0, PlayerTransportPhase::Playing, 5.0);
    playing.playback_rate = Some(1.25);
    playing.cache_buffering_percent = Some(60.0);
    playing.buffered_ahead_seconds = Some(9.0);
    coordination.observe_transport(playing, 6.0);
    let connected = coordination
        .take_participant_status_report(&session, true, 6.0)
        .unwrap();
    assert_eq!(
        connected.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(connected.position_seconds, Some(5.0));
    assert_eq!(connected.playback_rate, Some(1.25));
    assert_eq!(connected.cache_percent, Some(60.0));
    assert_eq!(connected.buffered_ahead_seconds, Some(9.0));

    let stalled = coordination
        .take_participant_status_report(
            &session,
            true,
            6.0 + PARTICIPANT_STATUS_TRANSPORT_TELEMETRY_STALE_SECONDS + 0.1,
        )
        .unwrap();
    assert_eq!(
        stalled.player_connection,
        ParticipantPlayerConnection::Unavailable
    );
    assert_eq!(stalled.position_seconds, None);
    assert_eq!(stalled.buffered_ahead_seconds, None);

    coordination
        .coordinator
        .set_steady_state_skew_seconds_for_test(-3.0);
    let sparse_recovery = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(11.2)),
    )
    .with_phase(PlayerTransportPhase::Playing);
    coordination.observe_transport(sparse_recovery, 11.2);
    let sparse = coordination
        .take_participant_status_report(&session, false, 11.2)
        .expect("the first post-gap sample must force an immediate fresh report");
    assert_eq!(
        sparse.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(sparse.position_seconds, None);
    assert_eq!(sparse.logical_paused, None);
    assert_eq!(sparse.playback_rate, None);
    assert_eq!(sparse.cache_percent, None);
    assert_eq!(sparse.buffered_ahead_seconds, None);
    assert_eq!(
        coordination.coordinator.metrics().steady_state_skew_seconds,
        None,
        "post-gap sparse telemetry must not revive prior skew"
    );

    let mut complete_recovery = transport(1, 11.3, PlayerTransportPhase::Playing, 6.0);
    complete_recovery.playback_rate = Some(1.1);
    complete_recovery.cache_buffering_percent = Some(20.0);
    complete_recovery.buffered_ahead_seconds = Some(4.0);
    coordination.observe_transport(complete_recovery, 11.3);
    let complete = coordination
        .take_participant_status_report(&session, true, 11.3)
        .expect("later explicit telemetry should remain reportable");
    assert_eq!(complete.position_seconds, Some(6.0));
    assert_eq!(complete.logical_paused, Some(false));
    assert_eq!(complete.playback_rate, Some(1.1));
    assert_eq!(complete.cache_percent, Some(20.0));
    assert_eq!(complete.buffered_ahead_seconds, Some(4.0));

    assert!(
        coordination
            .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 12.0)
    );
    assert!(
        !coordination
            .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 12.1)
    );
    let unavailable_snapshot = coordination.snapshot();
    let unavailable_bindings = coordination.adapter_generation_bindings.clone();
    let unavailable_pending_identity = coordination.pending_media_identity;
    let unavailable_clock = (
        coordination.adapter_clock_offset_seconds,
        coordination.last_external_now_seconds,
        coordination.last_coordinator_now_seconds,
    );
    let unavailable_barrier_state = (
        coordination.barrier.last_reported_barrier_ready,
        coordination.barrier.last_reported_barrier_started,
        coordination.barrier.last_reported_room_buffering,
    );
    let raced_actions = coordination
        .observe_transport(transport(2, 12.2, PlayerTransportPhase::Playing, 6.0), 12.2);
    assert!(raced_actions.is_empty());
    assert_eq!(coordination.snapshot(), unavailable_snapshot);
    assert_eq!(
        coordination.adapter_generation_bindings, unavailable_bindings,
        "a late detached-player sample must not bind its adapter generation"
    );
    assert_eq!(
        coordination.pending_media_identity,
        unavailable_pending_identity
    );
    assert_eq!(
        (
            coordination.adapter_clock_offset_seconds,
            coordination.last_external_now_seconds,
            coordination.last_coordinator_now_seconds,
        ),
        unavailable_clock,
        "a late detached-player sample must not advance the coordination clock"
    );
    assert_eq!(
        (
            coordination.barrier.last_reported_barrier_ready,
            coordination.barrier.last_reported_barrier_started,
            coordination.barrier.last_reported_room_buffering,
        ),
        unavailable_barrier_state,
        "a late detached-player sample must not mutate barrier projection state"
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 12.2)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Unavailable,
        "late telemetry cannot revive an explicitly unavailable player"
    );

    assert!(
        coordination.set_external_player_availability(ExternalPlayerAvailability::Failed, 12.25,)
    );
    let failed_snapshot = coordination.snapshot();
    let failed_actions = coordination.observe_transport(
        transport(2, 12.26, PlayerTransportPhase::Playing, 6.1),
        12.26,
    );
    assert!(failed_actions.is_empty());
    assert_eq!(coordination.snapshot(), failed_snapshot);
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 12.26)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Failed,
        "late telemetry cannot revive an explicitly failed player"
    );

    assert!(
        coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 12.3)
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 12.3)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Starting
    );
    coordination.observe_transport(transport(1, 12.4, PlayerTransportPhase::Playing, 6.2), 12.4);
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 12.4)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Connected
    );

    let old_epoch = coordination.adapter_epoch;
    let new_epoch = coordination.reset_adapter_epoch(13.0);
    assert_ne!(new_epoch, old_epoch);
    assert_eq!(
        coordination.projected_local_position_at(13.0, Some(6.2)),
        None,
        "a replacement adapter epoch must not reuse the retired adapter's legacy position"
    );
    coordination.observe_transport_at_epoch(
        transport(1, 13.1, PlayerTransportPhase::Playing, 7.0),
        13.1,
        old_epoch,
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 13.1)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Starting,
        "adapter reset and late old-epoch telemetry cannot retain Connected"
    );

    let no_telemetry = RuntimePlaybackCoordination::default()
        .take_participant_status_report(&session, true, 0.0)
        .unwrap();
    assert_eq!(
        no_telemetry.player_connection,
        ParticipantPlayerConnection::Unavailable
    );
}

#[test]
fn participant_status_heartbeat_handles_finite_clock_rollback_without_a_liveness_gap() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.set_external_player_availability(ExternalPlayerAvailability::Unavailable, 10.0);
    assert!(
        coordination
            .take_participant_status_report(&session, false, f64::INFINITY)
            .is_none(),
        "an invalid first owner timestamp must not commit a fingerprint or send clock"
    );
    assert!(
        coordination
            .participant_status
            .last_participant_status_fingerprint
            .is_none()
    );
    assert!(
        coordination
            .participant_status
            .last_participant_status_sent_at_seconds
            .is_none()
    );
    coordination
        .take_participant_status_report(&session, false, 10.0)
        .expect("the lifecycle transition should establish a fingerprint");

    assert!(
        coordination
            .pending_participant_status_report(&session, true, 10.5)
            .is_none(),
        "a half-interval must not emit a heartbeat"
    );
    coordination
        .participant_status
        .last_participant_status_sent_at_seconds = Some(f64::NEG_INFINITY);
    assert!(
        coordination
            .pending_participant_status_report(&session, true, 10.0)
            .is_none(),
        "a non-finite previous clock sample must fail closed"
    );
    coordination
        .participant_status
        .last_participant_status_sent_at_seconds = Some(10.0);
    let rollback = coordination
        .take_participant_status_report(&session, true, 9.0)
        .expect("a finite wall-clock rollback should emit once and rebase the cadence");
    assert_eq!(rollback.report_sequence, 2);
    assert!(
        coordination
            .pending_participant_status_report(&session, true, 9.5)
            .is_none(),
        "the rebased cadence must not spin after rollback"
    );
    assert!(
        coordination
            .pending_participant_status_report(&session, true, 10.0)
            .is_some(),
        "the exact rebased interval should emit"
    );
    assert!(
        coordination
            .pending_participant_status_report(&session, true, f64::INFINITY)
            .is_none(),
        "a non-finite current clock sample must fail closed"
    );
    assert!(
        coordination
            .pending_participant_status_report(&session, true, 10.0)
            .is_some(),
        "the exact one-second boundary should emit"
    );
}

#[test]
fn participant_status_owner_clock_rollback_requires_new_transport_evidence() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("participant-status-owner-clock").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    coordination.mark_transport_telemetry_available();
    coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 100.0);
    coordination.observe_transport(
        transport(1, 1.0, PlayerTransportPhase::Playing, 42.5),
        100.0,
    );
    let initial = coordination
        .take_participant_status_report(&session, true, 100.0)
        .unwrap();
    assert_eq!(
        initial.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(initial.position_seconds, Some(42.5));

    let rollback = coordination
        .take_participant_status_report(&session, true, 50.0)
        .unwrap();
    assert_eq!(
        rollback.player_connection,
        ParticipantPlayerConnection::Unavailable
    );
    assert_eq!(rollback.position_seconds, None);
    let catch_up = coordination
        .take_participant_status_report(&session, true, 100.0)
        .unwrap();
    assert_eq!(
        catch_up.player_connection,
        ParticipantPlayerConnection::Unavailable,
        "wall-clock catch-up alone must not rejuvenate pre-rollback evidence"
    );

    let sparse = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(2.0)),
    )
    .with_phase(PlayerTransportPhase::Playing);
    coordination.observe_transport(sparse, 50.0);
    let rebased = coordination
        .take_participant_status_report(&session, true, 50.0)
        .unwrap();
    assert_eq!(
        rebased.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(
        rebased.position_seconds, None,
        "the first post-rollback sparse sample must not inherit old precision"
    );
}

#[test]
fn participant_status_connecting_clock_rollback_cannot_rejuvenate_starting() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    assert!(
        coordination
            .set_external_player_availability(ExternalPlayerAvailability::Connecting, 100.0,)
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 100.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Starting,
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 50.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Unavailable,
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 100.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Unavailable,
        "wall-clock catch-up without a new lifecycle/sample must not revive Starting",
    );
}

#[test]
fn participant_status_connecting_and_telemetry_unavailable_fence_pre_transition_transport() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("participant-status-lifecycle-fence").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0);
    coordination.observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 12.0), 1.0);
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 1.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Connected
    );

    assert!(
        coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 2.0)
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 2.0)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Starting,
        "Connecting must immediately supersede a fresh prior observation"
    );
    assert!(coordination.latest_observation.is_none());
    assert_eq!(
        coordination.projected_local_position_at(2.0, Some(12.0)),
        None,
        "Connecting must fence the pre-transition legacy position as well as rich telemetry"
    );
    let stale_actions =
        coordination.observe_transport(transport(1, 1.5, PlayerTransportPhase::Playing, 99.0), 2.1);
    assert!(stale_actions.is_empty());
    assert!(coordination.latest_observation.is_none());
    assert_eq!(
        coordination.participant_status_player_availability(2.1),
        ParticipantPlayerConnection::Starting,
        "an in-flight observation from before the Connecting fence cannot revive Connected"
    );

    coordination.observe_transport(transport(1, 2.1, PlayerTransportPhase::Playing, 13.0), 2.1);
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 2.1)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Connected
    );

    assert!(
        coordination.set_external_player_availability(
            ExternalPlayerAvailability::TelemetryUnavailable,
            2.2,
        )
    );
    assert_eq!(
        coordination
            .take_participant_status_report(&session, true, 2.2)
            .unwrap()
            .player_connection,
        ParticipantPlayerConnection::Unavailable
    );
    coordination.observe_transport(transport(1, 2.3, PlayerTransportPhase::Playing, 14.0), 2.3);
    assert!(coordination.latest_observation.is_none());
    assert_eq!(
        coordination.participant_status_player_availability(2.3),
        ParticipantPlayerConnection::Unavailable,
        "TelemetryUnavailable remains a hard fence until a new Connecting lifecycle"
    );
}

#[test]
fn participant_status_room_switch_cancels_leased_status_and_suppresses_old_room_heartbeats() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 0.0)
        .expect("the initial status should queue");
    let leased = runtime
        .pending_protocol_line()
        .unwrap()
        .expect("the old-room status should be leased");

    assert!(runtime.run_set_room("room2").unwrap());
    assert!(
        runtime
            .playback_coordination
            .pending_participant_status_report(&runtime.session, true, 2.0)
            .is_none(),
        "the old room must remain suppressed until authoritative membership changes"
    );
    assert!(runtime.release_protocol_line(leased.lease()));
    let queued = runtime.flush_queued_protocol_messages();
    assert!(reports_in(queued.clone()).is_empty());
    assert!(queued.iter().any(|message| {
        matches!(message, ProtocolMessage::Set(set) if set.set.room.as_ref().is_some_and(|room| room.name == "room2"))
    }));

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room2"}}}}}"#,
            3.0,
        )
        .unwrap();
    let new_room_reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(new_room_reports.len(), 1);
    assert_eq!(
        new_room_reports[0].player_connection,
        ParticipantPlayerConnection::Unavailable
    );
}

#[test]
fn participant_status_room_switch_requires_the_requested_authoritative_membership() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );

    assert!(runtime.run_set_room("room1").unwrap());
    assert!(
        runtime
            .playback_coordination
            .pending_participant_status_report(&runtime.session, true, 1.0)
            .is_some(),
        "re-selecting the authoritative current room must not fence reports"
    );

    assert!(runtime.run_set_room("server-will-normalize-this").unwrap());
    assert!(
        runtime
            .playback_coordination
            .pending_participant_status_report(&runtime.session, true, 2.0)
            .is_none()
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"List":{"room1":{"alice":{"features":{"sorotteParticipantStatusV1":true}}}}}"#,
            3.0,
        )
        .unwrap();
    assert!(
        reports_in(runtime.flush_queued_protocol_messages()).is_empty(),
        "a delayed List for the old room must not release the destination fence"
    );
    assert!(
        runtime
            .playback_coordination
            .pending_participant_status_report(&runtime.session, true, 3.1)
            .is_none()
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"user":{"alice":{"room":{"name":"server-will-normalize-this"}}}}}"#,
            4.0,
        )
        .unwrap();
    assert_eq!(
        reports_in(runtime.flush_queued_protocol_messages()).len(),
        1,
        "only the authoritative destination membership may release the fence"
    );
}

#[test]
fn participant_status_reconnect_preserves_a_durable_room_switch_fence() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    assert!(runtime.run_set_room("room2").unwrap());
    runtime.begin_protocol_connection_generation();
    runtime.session_mut().reset_sync_state_for_reconnect();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            1.0,
        )
        .unwrap();

    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .as_deref(),
        Some("room2"),
    );
    assert!(
        reports_in(runtime.flush_queued_protocol_messages()).is_empty(),
        "replacement Hello in the old room must not publish behind the retained SetRoom",
    );
    assert!(
        runtime
            .playback_coordination
            .pending_participant_status_report(&runtime.session, true, 1.1)
            .is_none(),
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room2"}}}}}"#,
            2.0,
        )
        .unwrap();
    assert_eq!(
        reports_in(runtime.flush_queued_protocol_messages()).len(),
        1
    );
}

#[test]
fn participant_status_inactive_phases_cancel_unleased_and_leased_reports() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 0.0)
        .unwrap();
    runtime.session_mut().mark_disconnected();
    assert!(reports_in(runtime.flush_queued_protocol_messages()).is_empty());

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            1.0,
        )
        .unwrap();
    runtime
        .set_external_player_availability(ExternalPlayerAvailability::Connecting, 1.0)
        .unwrap();
    let leased = runtime
        .pending_protocol_line()
        .unwrap()
        .expect("replacement status should be leased");
    runtime.session_mut().mark_closing();
    assert!(runtime.release_protocol_line(leased.lease()));
    assert!(reports_in(runtime.flush_queued_protocol_messages()).is_empty());
}

#[test]
fn participant_status_withdraw_evidence_requires_an_active_negotiated_epoch() {
    fn session_with_status_support(supported: bool) -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_message_json(&format!(
                r#"{{"Hello":{{"username":"alice","room":{{"name":"room1"}},"version":"1.7.5","features":{{"sorotteParticipantStatusV1":{supported}}}}}}}"#
            ))
            .expect("participant-status Hello should apply");
        session
    }

    let mut active_unsupported = ClientRuntime::new(
        session_with_status_support(false),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    let (_, unsupported_transitions) = capture_client_lifecycle_transitions(|| {
        active_unsupported.session_mut().mark_connecting();
    });
    assert_eq!(
        unsupported_transitions
            .iter()
            .map(|transition| transition.transition)
            .collect::<Vec<_>>(),
        ["SESSION-CONNECT-001"],
        "an active connection without negotiated status support has no status authority to withdraw"
    );

    let mut inactive_supported_session = session_with_status_support(true);
    inactive_supported_session.mark_disconnected();
    let mut inactive_supported = ClientRuntime::new(
        inactive_supported_session,
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    let (_, inactive_transitions) = capture_client_lifecycle_transitions(|| {
        inactive_supported.session_mut().mark_connecting();
    });
    assert_eq!(
        inactive_transitions
            .iter()
            .map(|transition| transition.transition)
            .collect::<Vec<_>>(),
        ["SESSION-CONNECT-001"],
        "negotiated support from an inactive connection is no longer a live status epoch"
    );

    let mut active_supported = ClientRuntime::new(
        session_with_status_support(true),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    let (_, supported_transitions) = capture_client_lifecycle_transitions(|| {
        active_supported.session_mut().mark_connecting();
    });
    assert_eq!(
        supported_transitions
            .iter()
            .map(|transition| transition.transition)
            .collect::<Vec<_>>(),
        [
            "STATUS-WITHDRAW-001",
            "STATUS-UNAVAILABLE-001",
            "SESSION-CONNECT-001",
        ]
    );
    for status_transition in &supported_transitions[..2] {
        assert_eq!(status_transition.trigger, Trigger::Shutdown);
        assert_eq!(status_transition.disposition, Disposition::Applied);
        assert!(status_transition.identities.is_empty());
    }
}

#[test]
fn participant_status_reconnect_reset_cancels_unleased_and_leased_reports() {
    let mut unleased = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    unleased
        .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 0.0)
        .unwrap();
    unleased.session_mut().reset_sync_state_for_reconnect();
    assert!(reports_in(unleased.flush_queued_protocol_messages()).is_empty());

    let mut leased = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    leased
        .set_external_player_availability(ExternalPlayerAvailability::Unavailable, 0.0)
        .unwrap();
    let staged = leased
        .pending_protocol_line()
        .unwrap()
        .expect("the old-generation status should be leased");
    leased.session_mut().reset_sync_state_for_reconnect();
    assert!(leased.release_protocol_line(staged.lease()));
    assert!(
        reports_in(leased.flush_queued_protocol_messages()).is_empty(),
        "a failed leased write must not retry status after reconnect reset"
    );
}

#[test]
fn participant_status_missing_sample_age_omits_precise_evidence() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("participant-status-overflow-age").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0);
    coordination.observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 12.0), 1.0);
    coordination.last_transport_telemetry_received_at_seconds = Some(f64::MAX);
    coordination.participant_status_evidence_times.position = Some(-f64::MAX);

    let report = coordination
        .take_participant_status_report(&session, false, f64::MAX)
        .expect("the changed playing fingerprint should report");
    assert_eq!(
        report.player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(report.position_seconds, None);
    assert_eq!(report.position_sample_age_ms, None);
    assert_eq!(
        report.sample_age_ms, None,
        "overflowed elapsed time must never be serialized as a saturated finite age"
    );
}

#[test]
fn runtime_never_reports_when_server_did_not_negotiate_participant_status() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("legacy-server-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.player.transport_updates.push_back(transport(
        1,
        1.0,
        PlayerTransportPhase::Playing,
        5.0,
    ));
    runtime.drain_player_transport_coordination(1.0).unwrap();
    assert!(runtime.flush_queued_protocol_messages().is_empty());

    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let messages = runtime.flush_queued_protocol_messages();
    assert!(reports_in(messages).is_empty());
}

#[test]
fn participant_status_capability_withdrawal_cancels_an_unsent_report() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("participant-status-capability-withdrawal").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    assert!(
        runtime
            .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
            .expect("initial report should queue")
    );
    assert_eq!(
        reports_in(
            runtime
                .control
                .outbound_messages()
                .iter()
                .cloned()
                .collect()
        )
        .len(),
        1
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":false}}}"#,
        )
        .unwrap();
    let remaining = runtime.flush_queued_protocol_messages();
    assert!(
        reports_in(remaining.clone()).is_empty(),
        "withdrawn status must not survive in a coalesced State"
    );
    assert!(
        remaining.is_empty(),
        "the pure advisory frame should disappear"
    );
}

#[test]
fn participant_status_uses_only_applied_authoritative_room_scope_and_never_client_offset() {
    let logical_id = "authoritative-participant-status-media";
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(91, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                    .with_request_id("participant-status-operation")
                    .with_request_nonce(5),
            )
            .with_status(barrier_status(91, None, PlaybackBarrierPhase::Preparing)),
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(91, 12, 0.0, 0.0, 10.0))
            .with_status(barrier_status(
                91,
                Some(12),
                PlaybackBarrierPhase::Committed,
            )),
    );

    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.set_config(PlaybackCoordinatorConfig {
        recovery_policy: crate::playback_coordinator::RecoveryPolicy::PreserveContent,
        ..PlaybackCoordinatorConfig::default()
    });
    coordination.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
    coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);
    coordination.observe_transport(
        transport(1, 10.0, PlayerTransportPhase::Rebuffering, 8.0),
        10.0,
    );
    coordination.observe_transport(transport(1, 11.0, PlayerTransportPhase::Playing, 9.0), 11.0);
    coordination.observe_transport(
        transport(1, 12.0, PlayerTransportPhase::Playing, 10.0),
        12.0,
    );

    let report = coordination
        .take_participant_status_report(&session, true, 12.0)
        .expect("negotiated status report should be available");
    assert_eq!(
        report.playback_scope,
        Some(ParticipantPlaybackScope::new(91).with_state_revision(12))
    );
    assert!(matches!(
        report.phase,
        ParticipantPlaybackPhase::Loading | ParticipantPlaybackPhase::Playing
    ));
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            13.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(91, 13, 10.0, 0.0, 10.0))
            .with_status(barrier_status(
                91,
                Some(13),
                PlaybackBarrierPhase::Committed,
            )),
    );
    coordination.update_desired_from_session(&session, 13.0);
    assert_eq!(
        coordination.coordinator.metrics().steady_state_skew_seconds,
        None,
        "adopting a new revision must discard skew from the prior revision even when cached telemetry applies it"
    );
    let revision_advanced = coordination
        .take_participant_status_report(&session, true, 13.0)
        .expect("new room revision should report without stale offset");
    assert_eq!(
        revision_advanced.playback_scope, None,
        "a new wire revision must not be reported until current telemetry applies it"
    );
    let fresh_adoption = coordination.observe_transport(
        transport(1, 13.1, PlayerTransportPhase::Playing, 10.1),
        13.1,
    );
    assert!(
        fresh_adoption
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::RevisionApplied { .. }))
    );
    let fresh_current_revision = coordination
        .take_participant_status_report(&session, true, 13.1)
        .expect("fresh same-revision transport should report");
    assert_eq!(
        fresh_current_revision.playback_scope,
        Some(ParticipantPlaybackScope::new(91).with_state_revision(13))
    );

    coordination.observe_transport(
        transport(1, 20.0, PlayerTransportPhase::Rebuffering, 15.0),
        20.0,
    );
    coordination.observe_transport(
        transport(1, 21.0, PlayerTransportPhase::Playing, 16.0),
        21.0,
    );
    coordination.observe_transport(
        transport(1, 22.0, PlayerTransportPhase::Playing, 17.0),
        22.0,
    );
    let recomputed = coordination
        .take_participant_status_report(&session, true, 22.0)
        .expect("fresh recovery observations should recompute current-revision skew");
    assert_eq!(
        recomputed.playback_scope,
        Some(ParticipantPlaybackScope::new(91).with_state_revision(13))
    );

    coordination.prepare_media_with_intent(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        23.0,
    );
    let refreshed = coordination
        .take_participant_status_report(&session, true, 23.0)
        .expect("transport refresh should still report its bound room scope");
    assert_eq!(
        refreshed.playback_scope, None,
        "transport replacement must wait for a current observation before publishing scope"
    );
    assert_eq!(
        coordination.coordinator.metrics().steady_state_skew_seconds,
        None,
        "transport replacement must invalidate the old media offset"
    );

    coordination.prepare_media(
        LogicalMediaId::new("unrelated-replacement-media").unwrap(),
        MediaTransportKind::NetworkVod,
        24.0,
    );
    coordination.observe_transport(transport(2, 24.1, PlayerTransportPhase::Playing, 2.0), 24.1);
    coordination.last_applied_revision = Some(2);
    coordination
        .coordinator
        .set_steady_state_skew_seconds_for_test(4.0);
    let unrelated = coordination
        .take_participant_status_report(&session, true, 24.1)
        .expect("replacement media should still report local player state");
    assert_eq!(
        unrelated.playback_scope, None,
        "retained barrier history must not be paired with unrelated current telemetry"
    );
}

#[test]
fn participant_status_revision_applies_its_captured_room_scope() {
    let captured_scope = ParticipantStatusRoomScope {
        room: "room1".to_owned(),
        local_media_generation: 7,
        media_generation: 41,
        state_revision: Some(11),
        transport_revision: Some(3),
    };
    let newer_scope = ParticipantStatusRoomScope {
        room: "room1".to_owned(),
        local_media_generation: 7,
        media_generation: 41,
        state_revision: Some(12),
        transport_revision: Some(4),
    };
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination
        .participant_status
        .participant_status_desired_scope_bindings
        .insert((7, 9), captured_scope.clone());
    coordination
        .participant_status
        .participant_status_desired_scope_bindings
        .insert((7, 8), captured_scope.clone());
    coordination
        .participant_status
        .participant_status_desired_scope_bindings
        .insert((7, 10), newer_scope.clone());
    coordination
        .participant_status
        .participant_status_desired_scope_bindings
        .insert((8, 1), newer_scope.clone());
    coordination
        .participant_status
        .participant_status_room_scope = Some(newer_scope.clone());
    coordination.pending_forced_seek_revision = Some(9);

    coordination.record_observation_outcomes(&[PlaybackCoordinatorAction::RevisionApplied {
        media_generation: 7,
        state_revision: 9,
    }]);

    assert_eq!(
        coordination
            .participant_status
            .participant_status_applied_room_scope,
        Some(captured_scope),
        "an old desired revision must adopt the scope captured with that revision",
    );
    assert_ne!(
        coordination
            .participant_status
            .participant_status_applied_room_scope,
        Some(newer_scope),
        "a refreshed authoritative scope cannot relabel old transport evidence as exact",
    );
    assert_eq!(
        coordination
            .participant_status
            .participant_status_desired_scope_bindings
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![(7, 10), (8, 1)],
        "only applied-or-older bindings in the same local generation are retired",
    );
    assert_eq!(
        coordination.pending_forced_seek_revision, None,
        "the matching forced revision is retired at the same application boundary",
    );
}

#[test]
fn participant_status_sequence_resets_per_connection_and_room_change_forces_transition() {
    let mut session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.mark_transport_telemetry_available();

    let first = coordination
        .take_participant_status_report(&session, true, 0.0)
        .expect("first connection report should exist");
    let periodic = coordination
        .take_participant_status_report(&session, true, PARTICIPANT_STATUS_HEARTBEAT_SECONDS)
        .expect("due forced heartbeat should report");
    assert_eq!((first.report_sequence, periodic.report_sequence), (1, 2));
    assert!(
        coordination
            .take_participant_status_report(
                &session,
                false,
                PARTICIPANT_STATUS_HEARTBEAT_SECONDS + 0.1,
            )
            .is_none(),
        "unchanged state must not create another immediate report"
    );

    session
        .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
        .unwrap();
    let room_transition = coordination
        .take_participant_status_report(&session, false, PARTICIPANT_STATUS_HEARTBEAT_SECONDS + 0.2)
        .expect("room scope change should force an immediate report");
    assert_eq!(room_transition.report_sequence, 3);

    coordination.begin_protocol_connection_generation(&session);
    let replacement_connection = coordination
        .take_participant_status_report(&session, false, PARTICIPANT_STATUS_HEARTBEAT_SECONDS + 0.3)
        .expect("replacement connection should publish a fresh report");
    assert_eq!(replacement_connection.report_sequence, 1);
}

#[test]
fn runtime_connection_generation_discards_old_status_and_restarts_sequence() {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("participant-status-runtime-reconnect").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    assert!(
        runtime
            .set_external_player_availability(ExternalPlayerAvailability::Connecting, 0.0)
            .expect("the old-generation report should queue")
    );
    let old_reports = reports_in(
        runtime
            .control
            .outbound_messages()
            .iter()
            .cloned()
            .collect(),
    );
    assert_eq!(old_reports.len(), 1);
    assert_eq!(old_reports[0].report_sequence, 1);

    runtime.begin_protocol_connection_generation();
    assert!(
        runtime.control.outbound_messages().is_empty(),
        "connection replacement must discard the old connection-scoped report"
    );
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status
            .next_participant_status_sequence,
        0
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            0.1,
        )
        .expect("the replacement connection Hello should apply");
    let replacement_reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(
        replacement_reports.len(),
        1,
        "only the replacement connection's report may remain queued"
    );
    assert_eq!(replacement_reports[0].report_sequence, 1);
    assert_eq!(
        replacement_reports[0].player_connection,
        ParticipantPlayerConnection::Starting
    );
}

fn barrier_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("barrier-aware hello should apply");
    session
}

fn readiness_v2_session_with_intent(
    media_generation: u64,
    membership_epoch: u64,
    last_technical_report_sequence: u64,
    user_intent: UserReadinessIntent,
) -> ClientSession {
    let (user_intent, room_ready, start_gate_phase) = match user_intent {
        UserReadinessIntent::Ready => ("ready", true, "waitingForTechnicalReadiness"),
        UserReadinessIntent::NotReady => ("notReady", false, "waitingForIntent"),
    };
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("readiness V2 Hello should apply");
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"sorotteReadinessV2":{{"snapshot":{{"roomReadinessRevision":1,"mediaGeneration":{media_generation},"startGatePhase":{{"phase":"{start_gate_phase}","mediaGeneration":{media_generation}}},"pauseOwner":{{"owner":"readinessStartGate","mediaGeneration":{media_generation}}},"participants":{{"alice":{{"roomReadinessRevision":1,"membershipEpoch":{membership_epoch},"lastTechnicalReportSequence":{last_technical_report_sequence},"username":"alice","userIntent":"{user_intent}","userIntentRevision":1,"userIntentSource":{{"type":"initialization"}},"technicalState":{{"phase":"preparing","mediaGeneration":{media_generation}}},"participationRole":"required","roomReady":{room_ready},"startEligible":false}}}}}}}}}}}}"#
        ))
        .expect("readiness V2 snapshot should apply");
    session
}

fn readiness_v2_session(
    media_generation: u64,
    membership_epoch: u64,
    last_technical_report_sequence: u64,
) -> ClientSession {
    readiness_v2_session_with_intent(
        media_generation,
        membership_epoch,
        last_technical_report_sequence,
        UserReadinessIntent::Ready,
    )
}

fn controlled_session_with_authority(controller: bool) -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("controlled-room hello should apply");
    let controller_update = if controller {
        r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#
    } else {
        r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":false}}}}"#
    };
    session
        .apply_message_json(controller_update)
        .expect("local controller projection should apply");
    session
}

fn controlled_barrier_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("controlled barrier-aware hello should apply");
    assert_eq!(session.local_can_control(), Some(false));
    session
}

fn apply_barrier_extension(session: &mut ClientSession, extension: PlaybackBarrierSetExtension) {
    session
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(extension),
        ))
        .expect("barrier extension should apply");
}

fn transport(
    adapter_generation: u64,
    observed_at_seconds: f64,
    phase: PlayerTransportPhase,
    position_seconds: f64,
) -> PlayerTransportTelemetryUpdate {
    let mut update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(adapter_generation),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(
            observed_at_seconds,
        )),
    )
    .with_phase(phase)
    .with_position_seconds(position_seconds)
    .with_logical_pause(false);
    update.paused_for_cache = Some(matches!(phase, PlayerTransportPhase::Rebuffering));
    update.seeking = Some(matches!(phase, PlayerTransportPhase::Seeking));
    update.seekable = Some(true);
    update.core_idle = Some(false);
    update
}

fn paused_transport(
    adapter_generation: u64,
    observed_at_seconds: f64,
    phase: PlayerTransportPhase,
    position_seconds: f64,
) -> PlayerTransportTelemetryUpdate {
    let mut update = transport(
        adapter_generation,
        observed_at_seconds,
        phase,
        position_seconds,
    );
    update.logical_pause = Some(true);
    update.core_idle = Some(true);
    update
}

fn barrier_status(
    media_generation: u64,
    state_revision: Option<u64>,
    phase: PlaybackBarrierPhase,
) -> PlaybackBarrierStatusPayload {
    PlaybackBarrierStatusPayload {
        media_generation,
        state_revision,
        phase,
        policy: PlaybackBarrierPolicy::Controller,
        quorum: None,
        deadline: 100.0,
        participants: BTreeMap::new(),
        excluded_legacy_clients: BTreeSet::new(),
    }
}

fn has_pause_play_or_seek(actions: &[PlaybackCoordinatorAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(_)
                    | CoordinatorPlayerCommand::Play(_)
                    | CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )
    })
}

#[derive(Default)]
struct CoordinatedTestPlayer {
    transport_updates: VecDeque<PlayerTransportTelemetryUpdate>,
    command_progress_updates: VecDeque<sorotte_player_api::PlayerCommandProgress>,
    ordered_batches: VecDeque<PlayerEventBatch>,
    acknowledged_batches: Vec<PlayerEventAcknowledgementToken>,
    ordered_delivery: bool,
    reject_next_acknowledgement: bool,
    commands: Vec<PlayerCommand>,
    next_command_id: u64,
    advertises_telemetry: bool,
    reject_rate_commands: bool,
    reject_pause_commands: bool,
    reject_seek_commands: bool,
    ordered_batch_after_rejected_seek: Option<PlayerEventBatch>,
}

impl PlayerAdapter for CoordinatedTestPlayer {
    fn name(&self) -> &'static str {
        "coordinated-test-player"
    }

    fn capabilities(&self) -> PlayerCapabilities {
        if self.advertises_telemetry {
            PlayerCapabilities::from_capabilities([PlayerCapability::Telemetry])
        } else {
            PlayerCapabilities::NONE
        }
    }

    fn execute(&mut self, command: PlayerCommand) -> Result<(), PlayerError> {
        if self.reject_seek_commands && matches!(command, PlayerCommand::SetPosition(_)) {
            self.commands.push(command);
            if let Some(batch) = self.ordered_batch_after_rejected_seek.take() {
                self.ordered_batches.push_back(batch);
            }
            return Err(PlayerError::OperationFailed(
                "test player lost its active media before seek dispatch".to_owned(),
            ));
        }
        if self.reject_rate_commands && matches!(command, PlayerCommand::SetPlaybackRate(_)) {
            return Err(PlayerError::OperationFailed(
                "test player rejected rate cleanup".to_owned(),
            ));
        }
        if self.reject_pause_commands
            && matches!(
                command,
                PlayerCommand::SetPaused(_) | PlayerCommand::Play(_)
            )
        {
            self.commands.push(command);
            return Err(PlayerError::OperationFailed(
                "test player rejected pause/play command".to_owned(),
            ));
        }
        self.commands.push(command);
        Ok(())
    }

    fn execute_tracked(&mut self, command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        if self.reject_seek_commands && matches!(command, PlayerCommand::SetPosition(_)) {
            self.commands.push(command);
            if let Some(batch) = self.ordered_batch_after_rejected_seek.take() {
                self.ordered_batches.push_back(batch);
            }
            return Err(PlayerError::OperationFailed(
                "test player lost its active media before seek dispatch".to_owned(),
            ));
        }
        if self.reject_rate_commands && matches!(command, PlayerCommand::SetPlaybackRate(_)) {
            return Err(PlayerError::OperationFailed(
                "test player rejected tracked rate cleanup".to_owned(),
            ));
        }
        if self.reject_pause_commands
            && matches!(
                command,
                PlayerCommand::SetPaused(_) | PlayerCommand::Play(_)
            )
        {
            self.commands.push(command);
            return Err(PlayerError::OperationFailed(
                "test player rejected tracked pause/play command".to_owned(),
            ));
        }
        self.commands.push(command);
        self.next_command_id = self.next_command_id.saturating_add(1).max(1);
        Ok(PlayerCommandId::new(self.next_command_id))
    }

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        self.transport_updates.pop_front()
    }

    fn take_command_progress(&mut self) -> Option<sorotte_player_api::PlayerCommandProgress> {
        self.command_progress_updates.pop_front()
    }

    fn player_event_delivery_mode(&self) -> PlayerEventDeliveryMode {
        if self.ordered_delivery {
            PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        } else {
            PlayerEventDeliveryMode::LegacyTypedQueues
        }
    }

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        self.ordered_batches.front().cloned()
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        if self.reject_next_acknowledgement {
            self.reject_next_acknowledgement = false;
            return Err(PlayerError::OperationFailed(
                "test acknowledgement failure".to_owned(),
            ));
        }
        let Some(batch) = self.ordered_batches.front() else {
            return Err(PlayerError::OperationFailed(
                "no ordered batch is pending".to_owned(),
            ));
        };
        if batch.acknowledgement_token != token {
            return Err(PlayerError::OperationFailed(
                "wrong ordered batch acknowledgement".to_owned(),
            ));
        }
        self.acknowledged_batches.push(token);
        self.ordered_batches.pop_front();
        Ok(())
    }
}

fn ordered_runtime() -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    ClientRuntime::new(
        ClientSession::default(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    )
}

fn active_snapshot(
    epoch: PlayerAttachmentEpoch,
    through_sequence: u64,
    attempt_id: LoadAttemptId,
    media_generation: PlayerMediaGeneration,
    transport: PlayerTransportSnapshot,
) -> sorotte_player_api::PlayerAuthoritativeSnapshot {
    sorotte_player_api::PlayerAuthoritativeSnapshot {
        attachment_epoch: epoch,
        sequence_boundary: PlayerSequenceBoundary::new(epoch, through_sequence),
        transport,
        active_load: SnapshotField::Known(PlayerActiveLoadSnapshot {
            attempt_id,
            media_generation,
            command_id: None,
            playlist_entry_id: Some(10),
            physical_file_loaded: true,
            semantic_load_result: Some(PlayerLoadAttemptResult::Loaded),
            logical_ownership_revoked: false,
        }),
        current_playlist_entry_id: SnapshotField::Known(10),
        current_path: SnapshotField::Known("episode.mkv".to_owned()),
    }
}

fn ordered_batch(
    epoch: PlayerAttachmentEpoch,
    through_sequence: u64,
    token: u64,
    snapshot: Option<sorotte_player_api::PlayerAuthoritativeSnapshot>,
    events: Vec<SequencedPlayerEvent>,
    semantic_outcomes: Vec<SequencedPlayerSemanticOutcome>,
) -> PlayerEventBatch {
    PlayerEventBatch {
        attachment_epoch: epoch,
        sequence_boundary: PlayerSequenceBoundary::new(epoch, through_sequence),
        authoritative_snapshot: snapshot,
        events,
        semantic_outcomes,
        acknowledgement_token: PlayerEventAcknowledgementToken::new(epoch, token),
    }
}

fn ordered_playing_transport(
    media_generation: PlayerMediaGeneration,
    observed_at: PlayerObservationTimestamp,
    position_seconds: f64,
) -> PlayerTransportSnapshot {
    PlayerTransportSnapshot {
        load_attempt_id: SnapshotField::Known(LoadAttemptId::new(1)),
        media_generation: SnapshotField::Known(media_generation),
        observed_at: SnapshotField::Known(observed_at),
        phase: SnapshotField::Known(PlayerTransportPhase::Playing),
        position_seconds: SnapshotField::Known(position_seconds),
        playback_rate: SnapshotField::Known(1.0),
        logical_pause: SnapshotField::Known(false),
        paused_for_cache: SnapshotField::Known(false),
        seeking: SnapshotField::Known(false),
        core_idle: SnapshotField::Known(false),
        ..PlayerTransportSnapshot::default()
    }
}

fn ordered_paused_transport(
    media_generation: PlayerMediaGeneration,
    observed_at: PlayerObservationTimestamp,
    position_seconds: f64,
) -> PlayerTransportSnapshot {
    let mut transport = ordered_playing_transport(media_generation, observed_at, position_seconds);
    transport.phase = SnapshotField::Known(PlayerTransportPhase::ReadyPaused);
    transport.logical_pause = SnapshotField::Known(true);
    transport
}

#[test]
fn ordered_player_batch_emits_participant_status_without_waiting_for_heartbeat() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-participant-status").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    runtime
        .drain_player_transport_coordination(1.0)
        .expect("ordered snapshot should drain");
    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].report_sequence, 1);
    assert_eq!(reports[0].phase, ParticipantPlaybackPhase::Playing);
    assert_eq!(reports[0].position_seconds, Some(42.0));
}

#[test]
fn ordered_delivery_reconciles_new_room_authority_without_waiting_for_another_player_batch() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-room-authority-without-player-edge").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("initial ordered snapshot should drain");
    runtime.player.commands.clear();

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":42.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.1,
        )
        .expect("remote room authority should apply");
    assert!(
        runtime.player.ordered_batches.is_empty(),
        "the regression requires a room transition without another player edge"
    );

    runtime
        .drain_player_transport_coordination(1.1)
        .expect("an empty ordered drain should still reconcile room authority");

    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true))),
        "ordered delivery must not strand remote Pause until a future player event"
    );
}

#[test]
fn ordered_state_sync_drains_physical_seek_before_publishing_response() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-state-sync-seek-fence").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.8,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("the pre-seek snapshot should drain");
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":7.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
            1.1,
        )
        .expect("the canonical seek should apply");
    runtime
        .drain_player_transport_coordination(1.1)
        .expect("the canonical seek should reach the player");
    runtime.flush_queued_protocol_messages();

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        Some(active_snapshot(
            epoch,
            2,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1200)),
                7.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(7.0)
                    .with_paused(true)
                    .with_set_by("bob"),
            ),
            false,
            2.0,
        )
    );
    let response_position = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state
                .state
                .playstate
                .and_then(|playstate| playstate.position),
            _ => None,
        })
        .next_back();
    assert_eq!(
        response_position,
        Some(7.0),
        "an inbound State response must sample the acknowledged physical seek before publishing local authority"
    );
}

#[test]
fn adjacent_pause_then_seek_rejects_a_late_pre_pause_play_projection() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut session = participant_status_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice","sorotteTransportRevision":1}}}"#,
            1.0,
        )
        .expect("initial canonical pause should apply");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("adjacent-pause-seek-player-fence").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("initial paused snapshot should drain");
    runtime.flush_queued_protocol_messages();

    assert!(runtime.run_set_paused(false).expect("Play should dispatch"));
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice","sorotteTransportRevision":2}}}"#,
            1.1,
        )
        .expect("canonical Play echo should apply");
    assert!(runtime.run_set_paused(true).expect("Pause should dispatch"));
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.1,"paused":true,"doSeek":false,"setBy":"alice","sorotteTransportRevision":3}}}"#,
            1.2,
        )
        .expect("canonical Pause echo should apply");

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 2),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                load_attempt_id: Some(attempt_id),
                media_generation: Some(media_generation),
                observed_at: Some(PlayerObservationTimestamp::from_adapter_start(
                    Duration::from_millis(1150),
                )),
                phase: Some(PlayerTransportPhase::Playing),
                position_seconds: Some(0.1),
                logical_pause: Some(false),
                paused_for_cache: Some(false),
                ..PlayerTransportDelta::default()
            }),
        }],
        Vec::new(),
    ));

    assert!(
        runtime
            .run_seek_to_position(3.0)
            .expect("adjacent Seek should dispatch")
    );
    assert_eq!(
        runtime.session().local_paused(),
        Some(false),
        "the regression must actually drain the late Play projection before building Seek"
    );
    let seek_playstate = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state.state.playstate,
            _ => None,
        })
        .find(|playstate| {
            playstate.do_seek == Some(true)
                && playstate
                    .position
                    .is_some_and(|position| (position - 3.0).abs() < f64::EPSILON)
        })
        .expect("the local Seek must publish one causal playstate");
    assert_eq!(
        seek_playstate.paused,
        Some(true),
        "Seek must preserve canonical Pause despite the late pre-Pause player edge"
    );
}

#[test]
fn ordered_state_sync_never_pairs_new_revision_with_pre_effect_player_sample() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("same-frame-transport-revision-fence").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.8,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("the pre-seek player sample should drain");
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.8,"paused":true,"doSeek":false,"setBy":"bob","sorotteTransportRevision":1}}}"#,
            1.0,
        )
        .expect("the existing room authority should be established");

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(7.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("bob")
                    .with_transport_revision(2),
            ),
            false,
            1.1,
        )
    );
    let first = runtime.flush_queued_protocol_messages();
    assert!(first.iter().any(|message| matches!(
        message,
        ProtocolMessage::State(state)
            if state.state.playstate.is_none() && state.state.ping.is_some()
    )));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(7.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(2),
            ),
            false,
            1.15,
        )
    );
    let still_waiting = runtime.flush_queued_protocol_messages();
    assert!(
        still_waiting.iter().any(|message| matches!(
            message,
            ProtocolMessage::State(state)
                if state.state.playstate.is_none() && state.state.ping.is_some()
        )),
        "a later State tick must remain fenced while no post-command player sample exists"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        Some(active_snapshot(
            epoch,
            2,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1200)),
                7.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(7.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(2),
            ),
            false,
            1.2,
        )
    );
    let fresh_position = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .find_map(|message| match message {
            ProtocolMessage::State(state) => state
                .state
                .playstate
                .and_then(|playstate| playstate.position),
            _ => None,
        });
    assert_eq!(
        fresh_position,
        Some(7.0),
        "the same revision may publish only after post-command player evidence is available"
    );
}

#[test]
fn ordered_state_sync_fences_the_first_tagged_revision_until_player_evidence() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("first-transport-revision-fence").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(1),
            ),
            false,
            1.0,
        )
    );
    let first = runtime.flush_queued_protocol_messages();
    assert!(first.iter().any(|message| matches!(
        message,
        ProtocolMessage::State(state)
            if state.state.playstate.is_none() && state.state.ping.is_some()
    )));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(1),
            ),
            false,
            1.1,
        )
    );
    let still_waiting = runtime.flush_queued_protocol_messages();
    assert!(still_waiting.iter().any(|message| matches!(
        message,
        ProtocolMessage::State(state)
            if state.state.playstate.is_none() && state.state.ping.is_some()
    )));
    assert!(
        runtime.player().commands.iter().any(|command| matches!(
            command,
            PlayerCommand::SetPaused(false) | PlayerCommand::Play(_)
        )),
        "the canonical first Play must still reach the physical player"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        Some(active_snapshot(
            epoch,
            2,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1200)),
                0.2,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.2)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(1),
            ),
            false,
            1.2,
        )
    );
    let fresh_revision = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .find_map(|message| match message {
            ProtocolMessage::State(state) => state.state.playstate,
            _ => None,
        })
        .expect("post-command player evidence should release the first revision");
    assert_eq!(fresh_revision.paused, Some(false));
    assert_eq!(fresh_revision.transport_revision().unwrap(), Some(1));
}

#[test]
fn pre_baseline_local_play_rebases_onto_first_tagged_transport_state() {
    let mut session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("pre-baseline-local-play").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.stage_local_pause_intent(false, &session);

    let mutation_intent = coordination
        .active_local_pause_state_mutation_intent_for_inbound_transport(&session, Some(1), false)
        .expect("the pending Play should bind to the first non-seek revision");
    assert_eq!(mutation_intent.base_transport_revision, Some(1));

    let response = session.reconcile_state_and_build_response_at_with_pause_mutation_intent(
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(0.0)
                .with_paused(true)
                .with_do_seek(false)
                .with_set_by("bob")
                .with_transport_revision(1),
        ),
        0.0,
        false,
        0.0,
        0.0,
        1.0,
        Some(mutation_intent),
    );
    let playstate = response
        .playstate
        .expect("the pre-baseline Play must be emitted on the first revision");
    assert_eq!(playstate.paused, Some(false));
    assert_eq!(playstate.transport_revision().unwrap(), Some(1));
    assert_eq!(session.current_room_transport_revision(), Some(1));
    assert_eq!(
        coordination
            .active_local_pause_state_mutation_intent(&session)
            .map(|intent| intent.paused),
        Some(false),
        "the overlay must survive until a canonical server echo acknowledges it"
    );
}

#[test]
fn pre_baseline_local_play_does_not_override_a_first_canonical_seek() {
    let session = participant_status_session();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("pre-baseline-remote-seek").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.stage_local_pause_intent(false, &session);

    assert_eq!(
        coordination.active_local_pause_state_mutation_intent_for_inbound_transport(
            &session,
            Some(1),
            true,
        ),
        None,
        "a canonical seek edge must remain system-owned"
    );
    assert_eq!(
        coordination
            .pending_local_pause_intent
            .as_ref()
            .and_then(|intent| intent.base_transport_revision),
        None,
        "the superseded pre-baseline command must not be rebound to a seek revision"
    );
}

#[test]
fn ordered_local_pause_supersedes_unconsumed_play_revision_evidence() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("local-pause-supersedes-play-proof").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.8,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("the initial paused snapshot should drain");
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.8,"paused":true,"doSeek":false,"setBy":"bob","sorotteTransportRevision":1}}}"#,
            1.0,
        )
        .expect("the existing paused authority should be established");

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.8)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(2),
            ),
            false,
            1.1,
        )
    );
    let play_edge = runtime.flush_queued_protocol_messages();
    assert!(play_edge.iter().any(|message| matches!(
        message,
        ProtocolMessage::State(state)
            if state.state.playstate.is_none() && state.state.ping.is_some()
    )));

    // The physical Play is observed by the ordinary player drain, but no
    // later State tick has yet consumed that proof from the session fence.
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        Some(active_snapshot(
            epoch,
            2,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1200)),
                1.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.2)
        .expect("the physical Play should drain independently of State reconciliation");
    runtime.flush_queued_protocol_messages();
    assert_eq!(runtime.session().local_paused(), Some(false));

    runtime.stage_external_player_pause_intent(true, 1.25);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        3,
        Some(active_snapshot(
            epoch,
            3,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1300)),
                1.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(1.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob")
                    .with_transport_revision(2),
            ),
            false,
            1.3,
        )
    );
    let pause = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state.state.playstate,
            _ => None,
        })
        .find(|playstate| playstate.paused == Some(true))
        .expect("the newer local Pause must cross the same-revision evidence fence");
    assert_eq!(pause.transport_revision().unwrap(), Some(2));
    assert_ne!(pause.do_seek, Some(true));
}

#[test]
fn ordered_local_seek_preserves_an_adjacent_physical_pause() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-adjacent-pause-seek").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                3.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("the initial playing snapshot should drain");
    runtime.flush_queued_protocol_messages();

    // This acknowledged snapshot is available from the adapter, but the
    // ordinary runtime tick has not consumed it when the next user
    // command arrives.
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        Some(active_snapshot(
            epoch,
            2,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(1100)),
                3.4,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(runtime.run_seek_to_position(7.0).unwrap());
    let seek = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state.state.playstate,
            _ => None,
        })
        .find(|playstate| playstate.do_seek == Some(true))
        .expect("the explicit seek should be published");
    assert_eq!(
        seek.paused,
        Some(true),
        "Seek must sample the acknowledged Pause instead of reopening playback"
    );
}

#[test]
fn ordered_state_sync_defers_refresh_error_and_omits_stale_playstate() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-state-sync-refresh-error").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let media_generation = PlayerMediaGeneration::new(plan.media_generation);
    runtime.player.reject_next_acknowledgement = true;
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_paused_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                0.8,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(7.0)
                    .with_paused(true)
                    .with_set_by("bob"),
            ),
            false,
            2.0,
        )
    );
    let response = runtime
        .flush_queued_protocol_messages()
        .into_iter()
        .find_map(|message| match message {
            ProtocolMessage::State(state) => Some(state.state),
            _ => None,
        })
        .expect("the failed refresh should still return a ping-only State");
    assert!(response.playstate.is_none());
    assert!(response.ping.is_some());

    let error = runtime
        .drain_player_transport_coordination(2.1)
        .expect_err("the refresh error must remain visible to the runtime owner");
    assert_eq!(
        error,
        PlayerError::OperationFailed("test acknowledgement failure".to_owned())
    );
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
}

#[test]
fn ordered_player_batch_reports_status_before_returning_application_error() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            reject_pause_commands: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-participant-status-error").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    runtime
        .playback_coordination
        .coordinator
        .update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: plan.media_generation,
                state_revision: 1,
                paused: true,
                anchor_position_seconds: 42.0,
                anchor_observed_at_seconds: 1.0,
                force_seek: false,
            },
            DesiredRoomPlaybackUpdateKind::Ordinary,
        );
    let acknowledgement_token = PlayerEventAcknowledgementToken::new(epoch, 1);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    let error = runtime
        .drain_player_transport_coordination(1.0)
        .expect_err("the rejected pause command should remain the drain result");
    assert_eq!(
        error,
        PlayerError::OperationFailed("test player rejected tracked pause/play command".to_owned())
    );
    assert_eq!(
        runtime.player.acknowledged_batches,
        vec![acknowledgement_token],
        "the ordered batch should be acknowledged before its application error is returned"
    );

    let reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(
        reports.len(),
        1,
        "a phase-changing ordered batch must still queue participant status when applying it fails"
    );
    assert_eq!(reports[0].report_sequence, 1);
    assert_eq!(
        reports[0].player_connection,
        ParticipantPlayerConnection::Connected
    );
    assert_eq!(reports[0].position_seconds, Some(42.0));
}

#[test]
fn participant_status_ordered_terminal_records_precise_pause_evidence() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-participant-status-terminal").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );

    let mut initial_transport = ordered_playing_transport(
        media_generation,
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
        42.0,
    );
    initial_transport.logical_pause = SnapshotField::KnownAbsent;
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            initial_transport,
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("initial ordered snapshot should drain");
    let initial_reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(initial_reports.len(), 1);
    assert_eq!(initial_reports[0].logical_paused, None);

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        2,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::LoadAttemptTerminal {
                    attempt_id,
                    media_generation,
                    outcome: PlayerPhysicalLoadOutcome::Ended,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::LogicalPlaybackTerminal {
                    attempt_id,
                    media_generation,
                    outcome: PlayerPhysicalLoadOutcome::Ended,
                },
            },
        ],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("terminal ordered event should drain");

    let terminal_reports = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(terminal_reports.len(), 1);
    assert_eq!(terminal_reports[0].phase, ParticipantPlaybackPhase::Ended);
    assert_eq!(terminal_reports[0].logical_paused, Some(true));
    assert_eq!(terminal_reports[0].sample_age_ms, Some(0));
    assert_eq!(
        runtime.session().local_paused(),
        Some(true),
        "the ordered terminal edge must update the session's physical playback projection"
    );
    assert!(
        runtime.pending_natural_playback_completion.is_some(),
        "an owned semantic Ended event should retain one application progression intent"
    );
}

#[test]
fn natural_eof_overtaking_desync_seek_is_not_a_player_failure() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = natural_completion_playlist_runtime(0, "episode.mkv");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode.mkv","episode2.mkv"],"user":"alice","sorottePlaylistEpoch":3}}}"#,
        )
        .expect("the canonical playlist should match the observed active file");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":4}}}"#,
        )
        .expect("the canonical first row should be selected");
    runtime.player.ordered_delivery = true;
    runtime.prepare_playback_media(
        LogicalMediaId::new("episode.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                9.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("the initial active-media snapshot should drain");
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"}}}}}"#)
        .expect("the remote room member should be known");
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            1.0,
        )
        .expect("the remote room playstate should apply");

    runtime.player.reject_seek_commands = true;
    runtime.player.ordered_batch_after_rejected_seek = Some(ordered_batch(
        epoch,
        3,
        2,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::LoadAttemptTerminal {
                    attempt_id,
                    media_generation,
                    outcome: PlayerPhysicalLoadOutcome::Ended,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::LogicalPlaybackTerminal {
                    attempt_id,
                    media_generation,
                    outcome: PlayerPhysicalLoadOutcome::Ended,
                },
            },
        ],
        Vec::new(),
    ));

    runtime
        .run_desync_correction_if_needed(1.1, false, false, true)
        .expect("terminal EOF should supersede the losing desync seek");
    assert!(
        runtime
            .player
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPosition(_))),
        "the regression must exercise a seek that loses the EOF race"
    );
    assert_eq!(
        runtime.playback_coordination_snapshot().diagnostic,
        PlaybackDiagnostic::Ended
    );
    assert!(
        runtime.pending_natural_playback_completion.is_some(),
        "the terminal edge must remain available to the playlist lifecycle"
    );
    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .expect("the retained completion should advance the canonical playlist")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1)
    );
}

fn natural_completion_playlist_runtime(
    selected_index: i64,
    local_file_name: &str,
) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice","sorottePlaylistEpoch":1}}}"#,
        )
        .expect("playlist change should apply");
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"playlistIndex":{{"index":{selected_index},"user":"alice","sorottePlaylistEpoch":2}}}}}}"#
        ))
        .expect("playlist index should apply");
    session
        .apply_message_json(&format!(
            r#"{{"Set":{{"user":{{"alice":{{"file":{{"name":"{local_file_name}","duration":240.0}}}}}}}}}}"#
        ))
        .expect("local file should apply");
    ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    )
}

#[test]
fn natural_completion_advances_once_only_while_completed_file_is_canonical() {
    let mut runtime = natural_completion_playlist_runtime(0, "episode1.mkv");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the selected playlist should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(
            LocalFileUpdate::new("episode1.mkv").with_path("C:/library/show/episode1.mkv"),
        ),
    });

    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .expect("matching natural completion should advance")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1)
    );
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("natural completion should emit a guarded Set.playlistIndex");
    };
    let playlist_index = set
        .set
        .playlist_index
        .as_ref()
        .expect("natural completion should emit playlistIndex");
    assert_eq!(playlist_index.index_value(), Some(1));
    assert_eq!(playlist_index.expected_playlist_index(), Some(0));
    assert_eq!(playlist_index.expected_playlist_epoch(), Some(2));
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("replaying the cadence must be a no-op"),
        "one physical EOF must not advance two rows"
    );
}

#[test]
fn natural_completion_retains_legacy_unconditional_index_compatibility_without_epoch() {
    let mut runtime = natural_completion_playlist_runtime(0, "episode1.mkv");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("legacy index snapshot should apply");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the selected playlist should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    assert_eq!(
        runtime.session().current_room_playlist_canonical_epoch(),
        None
    );
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch: None,
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("episode1.mkv")),
    });

    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .expect("legacy natural completion should still advance")
    );
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("legacy natural completion should emit Set.playlistIndex");
    };
    let playlist_index = set
        .set
        .playlist_index
        .as_ref()
        .expect("legacy natural completion should emit playlistIndex");
    assert_eq!(playlist_index.index_value(), Some(1));
    assert!(
        !playlist_index.has_expected_playlist_state(),
        "a server that never issued an epoch must receive the legacy payload shape"
    );
}

#[test]
fn final_no_loop_natural_completion_publishes_one_bounded_terminal_pause() {
    let mut runtime = natural_completion_playlist_runtime(2, "episode3.mkv");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"State":{"playstate":{"position":239.5,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":9}}}"#,
        )
        .expect("canonical playing state should apply");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the final selection should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("episode3.mkv").with_duration_seconds(240.0)),
    });

    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .expect("the final completion should publish terminal authority")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(2),
        "a no-loop terminal pause must not mutate playlist selection"
    );
    let terminal = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state.state.playstate.as_ref(),
            _ => None,
        })
        .next()
        .expect("the final boundary should emit one State.playstate");
    assert_eq!(terminal.position, Some(240.0));
    assert_eq!(terminal.paused, Some(true));
    assert_eq!(terminal.do_seek, Some(false));
    assert_eq!(terminal.transport_revision().unwrap(), Some(9));
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("the consumed completion should not replay"),
        "one physical EOF must emit at most one terminal mutation"
    );
    assert_eq!(runtime.control().outbound_messages().len(), 1);
}

#[test]
fn controlled_room_noncontroller_cannot_publish_terminal_pause() {
    let mut session = controlled_session_with_authority(false);
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice","sorottePlaylistEpoch":1}}}"#,
        )
        .expect("controlled playlist should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":1,"user":"alice","sorottePlaylistEpoch":2}}}"#,
        )
        .expect("controlled final selection should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode2.mkv","duration":240.0}}}}}"#,
        )
        .expect("controlled local file should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":239.5,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":9}}}"#,
        )
        .expect("controlled canonical playing state should apply");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the controlled final selection should exist");
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision: runtime.session().current_room_playlist_selection_revision(),
        canonical_playlist_epoch: runtime.session().current_room_playlist_canonical_epoch(),
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("episode2.mkv").with_duration_seconds(240.0)),
    });

    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("an unauthorized terminal completion should be consumed safely")
    );
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(runtime.control().outbound_messages().is_empty());
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1)
    );
}

#[test]
fn natural_completion_uses_verified_path_when_published_name_is_lossy() {
    let mut runtime = natural_completion_playlist_runtime(0, "episode1.mkv");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["C:/library/show/episode1.mkv","C:/library/show/episode2.mkv"],"user":"alice","sorottePlaylistEpoch":3}}}"#,
        )
        .expect("absolute-path playlist should apply");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":4}}}"#,
        )
        .expect("absolute-path selection should apply");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the absolute-path playlist should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();

    assert!(
        runtime
            .session()
            .runtime_actions_for_local_playlist_next()
            .is_empty(),
        "the ordinary Next surface must retain its filename-projection guard"
    );
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(
            LocalFileUpdate::new("episode1.mkv").with_path("C:/library/show/episode1.mkv"),
        ),
    });

    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .expect("verified natural completion should advance"),
        "an exact physical path proof must not be discarded by a later basename-only check"
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1)
    );
    assert!(runtime.pending_natural_playback_completion.is_none());
}

#[test]
fn late_natural_completion_cannot_skip_a_selection_advanced_by_a_peer() {
    let mut runtime = natural_completion_playlist_runtime(0, "episode1.mkv");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the completed selection should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .expect("the peer playlist advance should apply");
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("episode1.mkv")),
    });

    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("stale completion should be consumed safely")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1),
        "a late EOF for row zero must not skip canonical row one"
    );
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn late_natural_completion_cannot_advance_after_same_row_reselection() {
    let mut runtime = natural_completion_playlist_runtime(0, "episode1.mkv");
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the completed selection should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#)
        .expect("the peer should be able to replay the current row");

    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .map(|playlist| (Some(playlist.revision), playlist.index)),
        Some((playlist_revision, playlist_index)),
        "same-row replay leaves the visible content revision and index unchanged"
    );
    assert_ne!(
        runtime.session().current_room_playlist_selection_revision(),
        playlist_selection_revision,
        "same-row replay must establish a new canonical selection generation"
    );

    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("episode1.mkv")),
    });

    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("the stale same-row completion should be consumed")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(0),
        "an EOF from the previous play of row zero must not advance its replay"
    );
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn late_natural_completion_cannot_skip_a_duplicate_playlist_target() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["same.mkv","same.mkv"],"user":"alice"}}}"#,
        )
        .expect("duplicate playlist should apply");
    session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("first duplicate row should be selected");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"same.mkv","duration":240.0}}}}}"#,
        )
        .expect("local file should apply");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    let (playlist_revision, playlist_index) = runtime
        .session()
        .current_room_playlist()
        .map(|playlist| (Some(playlist.revision), playlist.index))
        .expect("the completed duplicate selection should exist");
    let playlist_selection_revision = runtime.session().current_room_playlist_selection_revision();
    let canonical_playlist_epoch = runtime.session().current_room_playlist_canonical_epoch();
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .expect("peer should select the second duplicate row");
    runtime.pending_natural_playback_completion = Some(PendingNaturalPlaybackCompletion {
        attempt_id: Some(LoadAttemptId::new(4)),
        media_generation: Some(PlayerMediaGeneration::new(7)),
        playlist_revision,
        playlist_selection_revision,
        canonical_playlist_epoch,
        playlist_index,
        completed_file: Some(LocalFileUpdate::new("same.mkv")),
    });

    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .expect("the stale duplicate completion should be consumed")
    );
    assert_eq!(
        runtime
            .session()
            .current_room_playlist()
            .and_then(|playlist| playlist.index),
        Some(1),
        "matching display text must not turn a stale EOF into a second advance"
    );
    assert!(runtime.pending_natural_playback_completion.is_none());
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn failed_logical_terminal_never_becomes_playlist_progression_intent() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("active snapshot should drain");
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 2),
            event: PlayerEvent::LogicalPlaybackTerminal {
                attempt_id,
                media_generation,
                outcome: PlayerPhysicalLoadOutcome::Failed(
                    sorotte_player_api::PlayerMediaLoadFailureKind::Network,
                ),
            },
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("failed terminal should drain");

    assert!(runtime.pending_natural_playback_completion.is_none());
}

#[test]
fn ordered_playing_snapshot_seeds_fresh_position_projection() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-position-projection").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    assert_eq!(plan.media_generation, media_generation.get());
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    runtime
        .drain_player_transport_coordination(1.0)
        .expect("ordered snapshot should drain");

    assert_eq!(
        runtime.projected_local_position_at(1.0),
        Some(42.0),
        "a fresh ordered playing snapshot must seed the steady-state desync position projection"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 2),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                observed_at: Some(PlayerObservationTimestamp::from_adapter_start(
                    Duration::from_secs_f64(1.5),
                )),
                playback_rate: Some(2.0),
                ..PlayerTransportDelta::default()
            }),
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.5)
        .expect("ordered sparse rate transition should drain");
    assert_eq!(
        runtime.projected_local_position_at(1.75),
        Some(43.0),
        "an ordered sparse rate transition must close the old motion segment and project the new rate"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        3,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 3),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                observed_at: Some(PlayerObservationTimestamp::from_adapter_start(
                    Duration::from_secs(2),
                )),
                cache_percentage: Some(75.0),
                ..PlayerTransportDelta::default()
            }),
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("ordered non-position delta should drain");
    assert_eq!(
        runtime.projected_local_position_at(3.1),
        None,
        "a retained snapshot position must not be relabelled as a fresh actual sample by an unrelated ordered delta"
    );
}

#[test]
fn ordered_attachment_replacement_reports_starting_until_new_epoch_snapshot_has_telemetry() {
    let first_epoch = PlayerAttachmentEpoch::new(1);
    let replacement_epoch = PlayerAttachmentEpoch::new(2);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer {
            ordered_delivery: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-attachment-replacement-status").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        first_epoch,
        1,
        1,
        Some(active_snapshot(
            first_epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("initial ordered snapshot should drain");
    let connected = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(
        connected.last().unwrap().player_connection,
        ParticipantPlayerConnection::Connected
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        replacement_epoch,
        1,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(replacement_epoch, 1),
            event: PlayerEvent::AttachmentReplaced {
                previous_epoch: first_epoch,
            },
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("attachment replacement should drain");
    let starting = reports_in(runtime.flush_queued_protocol_messages());
    assert_eq!(starting.len(), 1);
    assert_eq!(
        starting[0].player_connection,
        ParticipantPlayerConnection::Starting
    );
    assert_eq!(starting[0].position_seconds, None);

    runtime.player.ordered_batches.push_back(ordered_batch(
        replacement_epoch,
        2,
        3,
        Some(sorotte_player_api::PlayerAuthoritativeSnapshot {
            attachment_epoch: replacement_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(replacement_epoch, 2),
            ..sorotte_player_api::PlayerAuthoritativeSnapshot::default()
        }),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.1)
        .expect("empty replacement snapshot should drain");
    assert_eq!(
        runtime
            .playback_coordination
            .participant_status_player_availability(2.1),
        ParticipantPlayerConnection::Starting,
        "an authoritative empty replacement snapshot cannot revive the retired epoch"
    );
}

#[test]
fn ordered_gap_marker_invalidates_position_projection_until_snapshot_recovery() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-gap-projection").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                42.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("initial ordered snapshot should drain");
    assert_eq!(runtime.projected_local_position_at(1.0), Some(42.0));

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 2),
            event: PlayerEvent::EventGapDetected,
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.1)
        .expect("gap marker is a valid acknowledged batch");

    assert_eq!(
        runtime.projected_local_position_at(1.1),
        None,
        "a gap means transport deltas were lost, so stale pre-gap position must stay fenced until an authoritative snapshot rebases the consumer"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        3,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 3),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                observed_at: Some(PlayerObservationTimestamp::from_adapter_start(
                    Duration::from_secs_f64(1.5),
                )),
                position_seconds: Some(99.0),
                ..PlayerTransportDelta::default()
            }),
        }],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.5)
        .expect("post-gap delta should be acknowledged but remain fenced");
    assert_eq!(
        runtime.projected_local_position_at(1.5),
        None,
        "ordinary deltas cannot re-authorize transport after a declared event gap"
    );
    assert_eq!(
        runtime.session().local_position_seconds(),
        None,
        "partial post-gap deltas must not leak into the session telemetry projection before \
         an authoritative snapshot"
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        4,
        4,
        Some(active_snapshot(
            epoch,
            4,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
                50.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("authoritative recovery snapshot should drain");
    assert_eq!(
        runtime.projected_local_position_at(2.0),
        Some(50.0),
        "an authoritative snapshot must clear the event-gap fence"
    );
}

#[test]
fn ordered_transport_maps_adapter_generation_to_pending_logical_media() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let adapter_generation = PlayerMediaGeneration::new(2);
    let mut runtime = ordered_runtime();
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("logical-generation-one-after-failed-physical-load").unwrap(),
        MediaTransportKind::NetworkVod,
        2.0,
    );
    assert_eq!(plan.media_generation, 1);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            adapter_generation,
            ordered_playing_transport(
                adapter_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
                20.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    runtime
        .drain_player_transport_coordination(2.0)
        .expect("ordered snapshot should drain");

    assert!(
        runtime
            .playback_coordination_snapshot()
            .transport_telemetry_observed,
        "physical adapter generation 2 must bind to pending logical generation 1 instead of being discarded by numeric comparison"
    );
    assert_eq!(
        runtime
            .playback_coordination
            .latest_observation
            .as_ref()
            .map(|observation| observation.media_generation),
        Some(plan.media_generation),
        "accepted ordered telemetry must be projected into the logical media generation"
    );
}

#[test]
fn ordered_transport_timestamp_preserves_queue_dwell() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("ordered-queue-dwell").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    assert_eq!(plan.media_generation, media_generation.get());
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_observation(
                    Duration::from_secs(1),
                    Duration::from_secs(5),
                ),
                10.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    runtime
        .drain_player_transport_coordination(105.0)
        .expect("ordered snapshot should drain");

    let mapped_observation_time = runtime
        .playback_coordination
        .latest_observation
        .as_ref()
        .expect("matching ordered telemetry should be retained")
        .observed_at_seconds;
    assert!(
        (mapped_observation_time - 101.0).abs() <= 0.000_001,
        "an observation sampled at adapter t=1 and delivered at adapter t=5/external t=105 must map to external t=101, got {mapped_observation_time}"
    );
}

#[test]
fn ordered_local_file_event_survives_ack_retry_and_publishes_once() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.reject_next_acknowledgement = true;
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        1,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 1),
                event: PlayerEvent::LoadAttemptStarting {
                    attempt_id,
                    media_generation,
                    command_id: None,
                    playlist_entry_id: 10,
                    owns_transport: true,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::LocalFileChanged {
                    attempt_id,
                    media_generation,
                    update: LocalFileUpdate::new("episode.mkv")
                        .with_duration_seconds(120.0)
                        .with_path("C:/media/episode.mkv"),
                },
            },
        ],
        Vec::new(),
    ));

    assert!(
        runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendRaw,
                PrivacyMode::SendRaw,
            )
            .is_err(),
        "the first ordered batch acknowledgement is deliberately rejected"
    );
    assert_eq!(
        runtime.pending_ordered_local_file_updates.pending().len(),
        1,
        "an applied effect must survive acknowledgement retry"
    );

    assert!(
        runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendRaw,
                PrivacyMode::SendRaw,
            )
            .expect("the replayed batch and file publication should succeed")
    );
    assert!(
        !runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendRaw,
                PrivacyMode::SendRaw,
            )
            .expect("an empty follow-up pump should be harmless")
    );

    let files = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            set.set.file.as_ref()
        })
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name.as_deref(), Some("episode.mkv"));
    assert_eq!(files[0].duration, Some(120.0));
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
    assert!(
        runtime
            .pending_ordered_local_file_updates
            .pending()
            .is_empty(),
        "successful privacy-aware publication acknowledges the local effect"
    );
}

#[test]
fn ordered_snapshot_preserves_matching_richer_pending_file_announcement() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        1,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 1),
                event: PlayerEvent::LoadAttemptStarting {
                    attempt_id,
                    media_generation,
                    command_id: None,
                    playlist_entry_id: 10,
                    owns_transport: true,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::LocalFileChanged {
                    attempt_id,
                    media_generation,
                    update: LocalFileUpdate::new("episode.mkv")
                        .with_duration_seconds(120.0)
                        .with_size_bytes(4096)
                        .with_path("C:/media/episode.mkv"),
                },
            },
        ],
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(1.0)
        .expect("ordered file event should drain");

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 3),
            event: PlayerEvent::EventGapDetected,
        }],
        Vec::new(),
    ));
    assert!(
        !runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendRaw,
                PrivacyMode::SendRaw,
            )
            .expect("gap handling should remain nonfatal"),
        "a pre-gap file announcement must remain fenced until the snapshot proves its identity"
    );
    assert!(
        runtime.control().outbound_messages().iter().all(|message| {
            !matches!(
                message,
                ProtocolMessage::Set(set) if set.set.file.is_some()
            )
        }),
        "no file identity may be published from an incomplete ordered event stream"
    );

    let mut snapshot = active_snapshot(
        epoch,
        4,
        attempt_id,
        media_generation,
        ordered_playing_transport(
            media_generation,
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
            10.0,
        ),
    );
    snapshot.current_path = SnapshotField::Known("C:/media/episode.mkv".to_owned());
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        4,
        3,
        Some(snapshot),
        Vec::new(),
        Vec::new(),
    ));
    runtime
        .drain_player_transport_coordination(2.0)
        .expect("authoritative snapshot should drain");

    let pending = runtime
        .pending_ordered_local_file_updates
        .front()
        .expect("the unreported matching file must remain pending");
    assert_eq!(
        runtime.pending_ordered_local_file_updates.pending().len(),
        1,
        "snapshot recovery should restore exactly one matching rich file announcement"
    );
    assert_eq!(pending.name, "episode.mkv");
    assert_eq!(pending.duration_seconds, Some(120.0));
    assert_eq!(pending.size_bytes, Some(4096));
    assert_eq!(pending.path.as_deref(), Some("C:/media/episode.mkv"));
}

#[test]
fn ordered_snapshot_recovers_local_file_announcement_after_lost_event() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("snapshot-file-recovery").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            ordered_playing_transport(
                media_generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
                10.0,
            ),
        )),
        Vec::new(),
        Vec::new(),
    ));

    assert!(
        runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendRaw,
                PrivacyMode::SendRaw,
            )
            .expect("snapshot recovery and file publication should succeed"),
        "an authoritative active-load snapshot is the recovery source when LocalFileChanged was dropped"
    );
    let published_files = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter(|message| {
            matches!(
                message,
                ProtocolMessage::Set(set) if set.set.file.is_some()
            )
        })
        .count();
    assert_eq!(published_files, 1);
}

#[test]
fn ordered_snapshot_rebase_clears_every_omitted_transport_field() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    let mut runtime = ordered_runtime();
    let plan = runtime.prepare_playback_media(
        LogicalMediaId::new("episode").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    assert_eq!(plan.media_generation, media_generation.get());

    let populated = PlayerTransportSnapshot {
        load_attempt_id: SnapshotField::Known(attempt_id),
        media_generation: SnapshotField::Known(media_generation),
        observed_at: SnapshotField::Known(PlayerObservationTimestamp::from_adapter_start(
            Duration::from_secs(1),
        )),
        phase: SnapshotField::Known(PlayerTransportPhase::Playing),
        position_seconds: SnapshotField::Known(42.0),
        playback_rate: SnapshotField::Known(1.25),
        logical_pause: SnapshotField::Known(false),
        paused_for_cache: SnapshotField::Known(true),
        cache_percentage: SnapshotField::Known(55.0),
        seeking: SnapshotField::Known(true),
        seekable: SnapshotField::Known(true),
        timeline_kind: SnapshotField::Known(sorotte_player_api::PlayerTimelineKind::SlidingLive),
        core_idle: SnapshotField::Known(false),
        demuxer_cache_idle: SnapshotField::Known(false),
        playback_restart_sequence: SnapshotField::Known(7),
        eof_reached: SnapshotField::Known(true),
        seekable_ranges: SnapshotField::Known(vec![sorotte_player_api::PlayerSeekableRange::new(
            10.0, 50.0,
        )]),
        known_live_seekable_window: SnapshotField::Known(
            sorotte_player_api::PlayerSeekableRange::new(10.0, 50.0),
        ),
        buffered_duration_seconds: SnapshotField::Known(8.0),
        buffered_bytes: SnapshotField::Known(4096),
        input_rate_bytes_per_second: SnapshotField::Known(2048),
        error_kind: SnapshotField::Known(sorotte_player_api::PlayerMediaLoadFailureKind::Network),
    };
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        1,
        Some(active_snapshot(
            epoch,
            1,
            attempt_id,
            media_generation,
            populated,
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(1.0).unwrap();
    assert_eq!(
        runtime
            .playback_coordination
            .latest_observation
            .as_ref()
            .and_then(|observation| observation.position_seconds),
        Some(42.0)
    );
    assert_eq!(runtime.session().local_paused(), Some(false));
    assert_eq!(runtime.session().local_position_seconds(), Some(42.0));
    assert_eq!(runtime.session().local_playback_rate(), Some(1.25));
    assert_eq!(runtime.session().local_paused_for_cache(), Some(true));
    assert_eq!(
        runtime.session().local_cache_buffering_percent(),
        Some(55.0)
    );
    assert!(
        runtime
            .session
            .model
            .playback
            .pending_cache_room_playstate_resync
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 2),
            event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                load_attempt_id: Some(attempt_id),
                media_generation: Some(media_generation),
                position_seconds: Some(43.0),
                logical_pause: Some(true),
                playback_rate: Some(1.0),
                paused_for_cache: Some(false),
                cache_percentage: Some(60.0),
                ..PlayerTransportDelta::default()
            }),
        }],
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(1.5).unwrap();
    assert_eq!(
        runtime
            .pending_player_playback_telemetry_updates
            .pending()
            .len(),
        1
    );

    let identity_only = PlayerTransportSnapshot {
        load_attempt_id: SnapshotField::Known(attempt_id),
        media_generation: SnapshotField::Known(media_generation),
        logical_pause: SnapshotField::KnownAbsent,
        playback_rate: SnapshotField::KnownAbsent,
        paused_for_cache: SnapshotField::KnownAbsent,
        ..PlayerTransportSnapshot::default()
    };
    let mut clearing = active_snapshot(epoch, 3, attempt_id, media_generation, identity_only);
    clearing.current_path = SnapshotField::KnownAbsent;
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        3,
        Some(clearing),
        Vec::new(),
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(2.0).unwrap();

    assert!(matches!(
        runtime.ordered_player_events.transport.position_seconds,
        SnapshotField::Unavailable
    ));
    assert!(matches!(
        runtime.ordered_player_events.transport.error_kind,
        SnapshotField::Unavailable
    ));
    assert!(matches!(
        runtime.ordered_player_events.transport.seekable_ranges,
        SnapshotField::Unavailable
    ));
    let observation = runtime
        .playback_coordination
        .latest_observation
        .as_ref()
        .unwrap();
    assert_eq!(observation.position_seconds, None);
    assert_eq!(observation.playback_rate, None);
    assert_eq!(observation.paused_for_cache, None);
    assert_eq!(observation.seeking, None);
    assert_eq!(observation.seekable, None);
    assert_eq!(observation.seekable_ranges, None);
    assert_eq!(observation.input_rate_bytes_per_second, None);
    assert!(runtime.last_local_file_update.is_none());
    assert_eq!(runtime.session().local_paused(), None);
    assert_eq!(runtime.session().local_position_seconds(), None);
    assert_eq!(runtime.session().local_playback_rate(), None);
    assert_eq!(runtime.session().local_paused_for_cache(), None);
    assert_eq!(runtime.session().local_cache_buffering_percent(), None);
    assert!(
        !runtime
            .session
            .model
            .playback
            .pending_cache_room_playstate_resync
    );
    assert_eq!(
        runtime
            .session
            .model
            .playback
            .cache_recovery_observation_position,
        None
    );
    assert!(
        !runtime
            .session
            .model
            .playback
            .cache_recovery_waiting_for_post_cache_position
    );
    assert!(
        runtime
            .pending_player_playback_telemetry_updates
            .pending()
            .is_empty()
    );
}

#[test]
fn ordered_transport_delta_acceptance_is_scoped_to_the_active_attempt() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let first_attempt = LoadAttemptId::new(1);
    let second_attempt = LoadAttemptId::new(2);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("same-logical-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let event = |sequence, event| SequencedPlayerEvent {
        order: PlayerEventOrder::new(epoch, sequence),
        event,
    };
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        5,
        1,
        None,
        vec![
            event(
                1,
                PlayerEvent::LoadAttemptActive {
                    attempt_id: first_attempt,
                    media_generation: generation,
                    command_id: None,
                    playlist_entry_id: 10,
                },
            ),
            event(
                2,
                PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(first_attempt),
                    media_generation: Some(generation),
                    position_seconds: Some(10.0),
                    ..PlayerTransportDelta::default()
                }),
            ),
            event(
                3,
                PlayerEvent::LoadAttemptActive {
                    attempt_id: second_attempt,
                    media_generation: generation,
                    command_id: None,
                    playlist_entry_id: 11,
                },
            ),
            event(
                4,
                PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(first_attempt),
                    media_generation: Some(generation),
                    position_seconds: Some(99.0),
                    ..PlayerTransportDelta::default()
                }),
            ),
            event(
                5,
                PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(second_attempt),
                    media_generation: Some(generation),
                    position_seconds: Some(20.0),
                    ..PlayerTransportDelta::default()
                }),
            ),
        ],
        Vec::new(),
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.load_attempt_id),
        Some(second_attempt)
    );
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.position_seconds),
        Some(20.0)
    );
    assert_eq!(
        runtime
            .playback_coordination
            .latest_observation
            .as_ref()
            .and_then(|observation| observation.position_seconds),
        Some(20.0)
    );
}

#[test]
fn ordered_batch_replay_after_ack_failure_is_idempotent() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.reject_next_acknowledgement = true;
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        9,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 1),
            event: PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation: generation,
                command_id: None,
                playlist_entry_id: 10,
            },
        }],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 2),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: None,
                requested_target: "episode".to_owned(),
                loaded_target: Some("episode".to_owned()),
                result: PlayerLoadAttemptResult::Loaded,
            }),
        }],
    ));

    assert!(runtime.drain_player_transport_coordination(1.0).is_err());
    assert_eq!(runtime.ordered_player_events.last_sequence, 2);
    assert_eq!(runtime.ordered_player_events.attempts.len(), 1);
    assert_eq!(
        runtime
            .ordered_player_events
            .applied_semantic_outcomes
            .len(),
        1,
        "failed acknowledgement must retain replay-only outcome identity"
    );
    runtime.drain_player_transport_coordination(2.0).unwrap();
    assert_eq!(runtime.ordered_player_events.last_sequence, 2);
    assert_eq!(runtime.ordered_player_events.attempts.len(), 1);
    assert_eq!(
        runtime
            .ordered_player_events
            .applied_semantic_outcomes
            .len(),
        0,
        "successful acknowledgement should compact replay-only outcome identity"
    );
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
    assert!(runtime.player.ordered_batches.is_empty());
}

#[cfg(feature = "test-support")]
#[test]
fn verification_seam_uses_production_batch_application_and_ack_compaction() {
    let epoch = PlayerAttachmentEpoch::new(3);
    let generation = PlayerMediaGeneration::new(7);
    let attempt_id = LoadAttemptId::new(11);
    let command_id = PlayerCommandId::new(13);
    let batch = ordered_batch(
        epoch,
        2,
        17,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 1),
                event: PlayerEvent::LoadAttemptStarting {
                    attempt_id,
                    media_generation: generation,
                    command_id: Some(command_id),
                    playlist_entry_id: 19,
                    owns_transport: true,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(attempt_id),
                    media_generation: Some(generation),
                    phase: Some(PlayerTransportPhase::Loading),
                    paused_for_cache: Some(true),
                    ..PlayerTransportDelta::default()
                }),
            },
        ],
        Vec::new(),
    );
    let mut runtime = ordered_runtime();

    assert_eq!(
        runtime
            .apply_ordered_player_event_batch_for_verification(&batch, 1.0)
            .unwrap(),
        None
    );
    let projection = runtime.lifecycle_verification_projection();
    assert_eq!(projection.attachment_epoch, SnapshotField::Known(epoch));
    assert_eq!(
        projection.sequence_boundary,
        SnapshotField::Known(batch.sequence_boundary)
    );
    assert_eq!(
        projection.in_flight_acknowledgement,
        SnapshotField::Known(batch.acknowledgement_token)
    );
    assert_eq!(
        projection.physical_transport_owner,
        SnapshotField::Known(attempt_id)
    );
    assert_eq!(
        projection.physical_media_generation,
        SnapshotField::Known(generation)
    );
    assert_eq!(
        projection.physical_playlist_entry_id,
        SnapshotField::Known(19)
    );
    assert_eq!(
        projection.transport.phase,
        SnapshotField::Known(PlayerTransportPhase::Loading)
    );
    let attempt = projection.attempts.get(&attempt_id).unwrap();
    assert_eq!(attempt.command_id, Some(command_id));
    assert_eq!(attempt.owns_transport, SnapshotField::Known(true));
    assert_eq!(attempt.semantic_load_result, SnapshotField::Known(None));
    assert_eq!(projection.pending_event_count, SnapshotField::Unavailable);
    assert_eq!(projection.physical_path, SnapshotField::Unavailable);
    assert_eq!(projection.logical_owner, SnapshotField::Unavailable);

    runtime.compact_acknowledged_player_event_batch_for_verification(
        PlayerEventAcknowledgementToken::new(epoch, 99),
        batch.sequence_boundary,
    );
    assert_eq!(
        runtime
            .lifecycle_verification_projection()
            .in_flight_acknowledgement,
        SnapshotField::Known(batch.acknowledgement_token),
        "a nonmatching external acknowledgement must not compact the consumer"
    );
    runtime.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
    assert_eq!(
        runtime
            .lifecycle_verification_projection()
            .in_flight_acknowledgement,
        SnapshotField::KnownAbsent
    );
}

#[cfg(feature = "test-support")]
#[test]
fn verification_projection_reports_only_semantic_facts_the_consumer_retains() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let loaded = LoadAttemptId::new(1);
    let superseded = LoadAttemptId::new(2);
    let indeterminate = LoadAttemptId::new(3);
    let mut consumer = OrderedPlayerEventConsumer::default();
    consumer.reset_for_epoch(epoch);
    consumer.install_attempt(
        loaded,
        OrderedLoadInstall {
            media_generation: generation,
            command_id: None,
            playlist_entry_id: Some(10),
            owns_transport: false,
            semantic_load_result: Some(PlayerLoadAttemptResult::Loaded),
            logical_ownership_revoked: false,
        },
    );
    consumer.install_attempt(
        superseded,
        OrderedLoadInstall {
            media_generation: generation,
            command_id: None,
            playlist_entry_id: Some(20),
            owns_transport: false,
            semantic_load_result: Some(PlayerLoadAttemptResult::Superseded),
            logical_ownership_revoked: false,
        },
    );
    consumer.revoke_logical_ownership(superseded, generation);
    consumer.install_attempt(
        indeterminate,
        OrderedLoadInstall {
            media_generation: generation,
            command_id: None,
            playlist_entry_id: Some(30),
            owns_transport: false,
            semantic_load_result: Some(PlayerLoadAttemptResult::Indeterminate),
            logical_ownership_revoked: false,
        },
    );
    consumer.mark_indeterminate(indeterminate, generation);

    let projection = consumer.lifecycle_verification_projection();
    assert_eq!(
        projection.attempts[&loaded].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
    );
    assert_eq!(
        projection.attempts[&superseded].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Superseded))
    );
    assert_eq!(
        projection.attempts[&indeterminate].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate)),
        "the consumer retains the reducer's exact semantic result"
    );
}

#[test]
fn starting_attempt_transport_is_accepted_only_with_explicit_physical_ownership() {
    for (owns_transport, expected_phase) in
        [(true, Some(PlayerTransportPhase::Loading)), (false, None)]
    {
        let epoch = PlayerAttachmentEpoch::new(1);
        let generation = PlayerMediaGeneration::new(1);
        let attempt_id = LoadAttemptId::new(1);
        let mut runtime = ordered_runtime();
        runtime.player.ordered_batches.push_back(ordered_batch(
            epoch,
            2,
            20 + u64::from(owns_transport),
            None,
            vec![
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch, 1),
                    event: PlayerEvent::LoadAttemptStarting {
                        attempt_id,
                        media_generation: generation,
                        command_id: Some(PlayerCommandId::new(9)),
                        playlist_entry_id: 10,
                        owns_transport,
                    },
                },
                SequencedPlayerEvent {
                    order: PlayerEventOrder::new(epoch, 2),
                    event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                        load_attempt_id: Some(attempt_id),
                        media_generation: Some(generation),
                        phase: Some(PlayerTransportPhase::Loading),
                        paused_for_cache: Some(true),
                        cache_percentage: Some(25.0),
                        ..PlayerTransportDelta::default()
                    }),
                },
            ],
            Vec::new(),
        ));

        runtime.drain_player_transport_coordination(1.0).unwrap();

        assert_eq!(
            runtime.ordered_player_events.transport_owner_attempt,
            owns_transport.then_some(attempt_id)
        );
        assert_eq!(
            snapshot_known_copy(&runtime.ordered_player_events.transport.phase),
            expected_phase
        );
        assert_eq!(
            snapshot_known_copy(&runtime.ordered_player_events.transport.paused_for_cache),
            owns_transport.then_some(true)
        );
    }
}

#[test]
fn same_generation_successor_start_accepts_transport_before_predecessor_terminal() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(7);
    let predecessor = LoadAttemptId::new(1);
    let successor = LoadAttemptId::new(2);
    let mut runtime = ordered_runtime();
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        4,
        24,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 1),
                event: PlayerEvent::LoadAttemptActive {
                    attempt_id: predecessor,
                    media_generation: generation,
                    command_id: None,
                    playlist_entry_id: 10,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::LoadAttemptStarting {
                    attempt_id: successor,
                    media_generation: generation,
                    command_id: None,
                    playlist_entry_id: 20,
                    owns_transport: true,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 4),
                event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(successor),
                    media_generation: Some(generation),
                    phase: Some(PlayerTransportPhase::Prebuffering),
                    paused_for_cache: Some(true),
                    ..PlayerTransportDelta::default()
                }),
            },
        ],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 2),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id: predecessor,
                media_generation: generation,
                command_id: None,
                requested_target: "stream".to_owned(),
                loaded_target: Some("stream".to_owned()),
                result: PlayerLoadAttemptResult::Superseded,
            }),
        }],
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();

    assert_eq!(
        runtime.ordered_player_events.transport_owner_attempt,
        Some(successor)
    );
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.phase),
        Some(PlayerTransportPhase::Prebuffering)
    );
    assert_eq!(
        runtime
            .ordered_player_events
            .attempts
            .get(&predecessor)
            .map(|binding| (binding.logical_ownership_revoked, binding.physical_terminal,)),
        Some((true, false))
    );
}

#[test]
fn superseded_late_active_does_not_clear_timeout_failure_readiness() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("replacement.mkv").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime
        .playback_coordination
        .last_technical_readiness_fingerprint = Some(TechnicalReadinessFingerprint {
        connection_generation: 0,
        membership_epoch: 1,
        media_generation: 1,
        authoritative_playback_revision: None,
        phase: TechnicalPlayabilityPhase::TemporarilyBlocked,
        reason: Some(TechnicalBlockCause::PlayerFailure),
        recovery: Some(RecoveryStage::NotStarted),
    });
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        25,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 1),
                event: PlayerEvent::LoadAttemptBound {
                    attempt_id,
                    media_generation: generation,
                    command_id: Some(PlayerCommandId::new(9)),
                    playlist_entry_id: 10,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::LoadAttemptActive {
                    attempt_id,
                    media_generation: generation,
                    command_id: Some(PlayerCommandId::new(9)),
                    playlist_entry_id: 10,
                },
            },
        ],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 2),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                requested_target: "superseded.mkv".to_owned(),
                loaded_target: None,
                result: PlayerLoadAttemptResult::Superseded,
            }),
        }],
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();

    assert_eq!(
        runtime.ordered_player_events.transport_owner_attempt,
        Some(attempt_id),
        "supersession revokes logical success without inventing a physical terminal"
    );
    assert!(
        runtime
            .playback_coordination
            .last_technical_readiness_fingerprint
            .is_some_and(|fingerprint| {
                fingerprint.reason == Some(TechnicalBlockCause::PlayerFailure)
            }),
        "a superseded physical attempt must not recover logical readiness"
    );
}

#[test]
fn loaded_semantic_outcome_for_quiescent_start_does_not_claim_transport_ownership() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        21,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 1),
            event: PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                playlist_entry_id: 10,
                owns_transport: false,
            },
        }],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 2),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                requested_target: "superseded-physical-load".to_owned(),
                loaded_target: Some("superseded-physical-load".to_owned()),
                result: PlayerLoadAttemptResult::Loaded,
            }),
        }],
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();

    assert_eq!(runtime.ordered_player_events.transport_owner_attempt, None);
    assert!(
        runtime
            .ordered_player_events
            .attempts
            .contains_key(&attempt_id)
    );
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.load_attempt_id),
        None
    );
}

#[test]
fn indeterminate_load_outcome_preserves_binding_for_a_late_active_event() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("late-load.mkv").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime
        .playback_coordination
        .last_technical_readiness_fingerprint = Some(TechnicalReadinessFingerprint {
        connection_generation: 0,
        membership_epoch: 1,
        media_generation: 1,
        authoritative_playback_revision: None,
        phase: TechnicalPlayabilityPhase::TemporarilyBlocked,
        reason: Some(TechnicalBlockCause::PlayerFailure),
        recovery: Some(RecoveryStage::NotStarted),
    });
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        2,
        22,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 1),
            event: PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                playlist_entry_id: 10,
                owns_transport: true,
            },
        }],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 2),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                requested_target: "late-load.mkv".to_owned(),
                loaded_target: None,
                result: PlayerLoadAttemptResult::Indeterminate,
            }),
        }],
    ));
    runtime.drain_player_transport_coordination(1.0).unwrap();

    assert_eq!(
        runtime.ordered_player_events.transport_owner_attempt,
        Some(attempt_id)
    );
    assert_eq!(
        runtime
            .ordered_player_events
            .attempts
            .get(&attempt_id)
            .map(|binding| (
                binding.owns_transport,
                binding.semantic_load_result,
                binding.physical_terminal,
            )),
        Some((true, Some(PlayerLoadAttemptResult::Indeterminate), false))
    );

    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        3,
        23,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 3),
            event: PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(9)),
                playlist_entry_id: 10,
            },
        }],
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(2.0).unwrap();

    assert_eq!(
        runtime.ordered_player_events.transport_owner_attempt,
        Some(attempt_id)
    );
    assert_eq!(
        runtime
            .playback_coordination
            .last_technical_readiness_fingerprint,
        None,
        "late positive load evidence must clear the stale timeout failure fingerprint"
    );
    assert_eq!(
        runtime
            .ordered_player_events
            .attempts
            .get(&attempt_id)
            .and_then(|binding| binding.semantic_load_result),
        Some(PlayerLoadAttemptResult::Indeterminate),
        "late physical activation must not rewrite the write-once semantic result"
    );
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.load_attempt_id),
        None
    );
}

#[test]
fn acknowledged_terminal_batch_compacts_consumer_replay_state() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        1,
        12,
        None,
        Vec::new(),
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 1),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: None,
                requested_target: "retired-private-target".to_owned(),
                loaded_target: None,
                result: PlayerLoadAttemptResult::NeverStarted,
            }),
        }],
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();

    assert!(runtime.ordered_player_events.attempts.is_empty());
    assert!(
        runtime
            .ordered_player_events
            .applied_semantic_outcomes
            .is_empty()
    );
    assert_eq!(
        runtime.ordered_player_events.acknowledged_semantic_sequence,
        1
    );
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
}

#[test]
fn gap_snapshot_rebases_events_but_retains_covered_semantic_outcomes() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("gap-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let snapshot = active_snapshot(
        epoch,
        5,
        attempt_id,
        generation,
        PlayerTransportSnapshot {
            load_attempt_id: SnapshotField::Known(attempt_id),
            media_generation: SnapshotField::Known(generation),
            position_seconds: SnapshotField::Known(30.0),
            ..PlayerTransportSnapshot::default()
        },
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        5,
        1,
        Some(snapshot),
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(epoch, 3),
            event: PlayerEvent::EventGapDetected,
        }],
        vec![SequencedPlayerSemanticOutcome {
            order: PlayerEventOrder::new(epoch, 4),
            outcome: PlayerSemanticOutcome::LoadAttempt(sorotte_player_api::LoadAttemptOutcome {
                attachment_epoch: epoch,
                attempt_id,
                media_generation: generation,
                command_id: None,
                requested_target: "gap-media".to_owned(),
                loaded_target: Some("gap-media".to_owned()),
                result: PlayerLoadAttemptResult::Loaded,
            }),
        }],
    ));

    runtime.drain_player_transport_coordination(1.0).unwrap();
    assert_eq!(runtime.ordered_player_events.last_sequence, 5);
    assert!(
        runtime
            .ordered_player_events
            .semantic_outcome_was_applied(PlayerEventOrder::new(epoch, 4))
    );
    assert_eq!(
        runtime.ordered_player_events.transport_owner_attempt,
        Some(attempt_id)
    );
    assert_eq!(
        snapshot_known_copy(&runtime.ordered_player_events.transport.position_seconds),
        Some(30.0)
    );
}

#[test]
fn ordered_batches_reject_stale_attachments_and_uncovered_sequence_gaps_transactionally() {
    let current_epoch = PlayerAttachmentEpoch::new(2);
    let stale_epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let mut runtime = ordered_runtime();
    runtime.prepare_playback_media(
        LogicalMediaId::new("fenced-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.player.ordered_batches.push_back(ordered_batch(
        current_epoch,
        1,
        1,
        Some(active_snapshot(
            current_epoch,
            1,
            attempt_id,
            generation,
            PlayerTransportSnapshot {
                load_attempt_id: SnapshotField::Known(attempt_id),
                media_generation: SnapshotField::Known(generation),
                position_seconds: SnapshotField::Known(10.0),
                ..PlayerTransportSnapshot::default()
            },
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(1.0).unwrap();

    runtime.player.ordered_batches.push_back(ordered_batch(
        stale_epoch,
        2,
        2,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(stale_epoch, 2),
            event: PlayerEvent::EventGapDetected,
        }],
        Vec::new(),
    ));
    assert!(runtime.drain_player_transport_coordination(2.0).is_err());
    assert_eq!(
        runtime.ordered_player_events.attachment_epoch,
        Some(current_epoch)
    );
    assert_eq!(runtime.ordered_player_events.last_sequence, 1);
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
    runtime.player.ordered_batches.pop_front();

    runtime.player.ordered_batches.push_back(ordered_batch(
        current_epoch,
        3,
        3,
        None,
        vec![SequencedPlayerEvent {
            order: PlayerEventOrder::new(current_epoch, 3),
            event: PlayerEvent::EventGapDetected,
        }],
        Vec::new(),
    ));
    assert!(runtime.drain_player_transport_coordination(3.0).is_err());
    assert_eq!(
        runtime.ordered_player_events.attachment_epoch,
        Some(current_epoch)
    );
    assert_eq!(runtime.ordered_player_events.last_sequence, 1);
    assert_eq!(runtime.player.acknowledged_batches.len(), 1);
}

fn reconnect_runtime(
    room_paused: bool,
    room_position: f64,
    local_paused: bool,
    local_position: f64,
) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            &format!(
                r#"{{"State":{{"playstate":{{"position":{room_position},"paused":{room_paused},"doSeek":false,"setBy":"bob"}}}}}}"#
            ),
            10.0,
        )
        .unwrap();
    session.model.playback.local_paused = Some(local_paused);
    session.model.playback.local_position = Some(local_position);
    session.model.reconnect.state_restore_validation_pending = true;

    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("reconnect-media").unwrap(),
        MediaTransportKind::NetworkVod,
        9.0,
    );
    runtime
}

fn runtime_with_tracked_fetch_seek() -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime
        .playback_coordination
        .coordinator
        .set_config(PlaybackCoordinatorConfig {
            command_timeout_seconds: 1.0,
            seek_preparation_timeout_seconds: 20.0,
            ..PlaybackCoordinatorConfig::default()
        });
    let generation = runtime
        .playback_coordination
        .prepare_media(
            LogicalMediaId::new("tracked-slow-fetch").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    runtime
        .playback_coordination
        .coordinator
        .update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: generation,
                state_revision: 1,
                paused: false,
                anchor_position_seconds: 40.0,
                anchor_observed_at_seconds: 0.0,
                force_seek: true,
            },
            DesiredRoomPlaybackUpdateKind::ExplicitSeek,
        );
    let mut initial = paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0);
    initial.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
        0.0, 10.0,
    )]);
    let actions = runtime
        .playback_coordination
        .observe_transport(initial, 0.1);
    runtime
        .execute_playback_coordinator_actions(actions, 0.1)
        .unwrap();
    assert_eq!(
        runtime.player().commands,
        vec![PlayerCommand::SetPosition(40.0)]
    );
    runtime
}

fn controlled_runtime_after_reconnect_without_fresh_authority(
    logical_media_id: &str,
) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut session = controlled_session_with_authority(true);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new(logical_media_id).unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.reconcile_external_player_playback(0.0);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );

    runtime.begin_protocol_connection_generation();
    runtime.session_mut().reset_sync_state_for_reconnect();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
            1.0,
        )
        .unwrap();
    assert_eq!(
        runtime.session().local_can_control(),
        Some(true),
        "Hello restores the cached controller projection, which is deliberately not fresh authority"
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.05,
        )
        .unwrap();
    let reconciliation = runtime.reconcile_external_player_playback(1.05);
    for action in reconciliation {
        if let PlaybackCoordinatorAction::Execute { command_id, .. } = action {
            runtime.report_external_coordinator_command_dispatch(command_id, Ok(()), 1.05);
        }
    }
    runtime.observe_external_player_transport(
        paused_transport(1, 1.075, PlayerTransportPhase::ReadyPaused, 10.0),
        1.075,
    );
    runtime
}

fn coordination_with_owned_catchup_rate() -> RuntimePlaybackCoordination {
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("catchup-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut session = ClientSession::default();
    session.model.connection.username = Some("alice".to_owned());
    session.model.room.name = Some("room".to_owned());
    session.model.room.playstates.insert(
        "room".to_owned(),
        RoomPlaystateView {
            paused: Some(false),
            position: Some(0.0),
            set_by: Some("bob".to_owned()),
            ..RoomPlaystateView::default()
        },
    );
    session
        .model
        .room
        .playstate_updated_at_seconds
        .insert("room".to_owned(), 0.0);
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
    coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);
    coordination.observe_transport(
        transport(1, 10.0, PlayerTransportPhase::Rebuffering, 8.0),
        10.0,
    );
    coordination.observe_transport(transport(1, 11.0, PlayerTransportPhase::Playing, 9.0), 11.0);
    coordination.observe_transport(
        transport(1, 12.0, PlayerTransportPhase::Playing, 10.0),
        12.0,
    );
    let mut catchup = transport(1, 13.0, PlayerTransportPhase::Playing, 11.0);
    catchup.playback_rate = Some(1.03);
    coordination.observe_transport(catchup, 13.0);
    assert!(coordination.snapshot().recovery_episode.is_some());
    coordination
}

#[test]
fn external_transport_rate_updates_legacy_correction_ownership() {
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.session.model.playback.local_playback_rate = Some(0.95);
    let mut observed = transport(1, 0.0, PlayerTransportPhase::Playing, 0.0);
    observed.playback_rate = Some(1.0);

    runtime.observe_external_player_transport(observed, 0.0);

    assert_eq!(
        runtime.session.model.playback.local_playback_rate,
        Some(1.0),
        "an external coordinator/mpv rate reset must re-arm legacy drift correction"
    );
}

#[test]
fn logical_media_identity_does_not_collapse_url_queries_or_basenames() {
    assert!(!logical_media_ids_match(
        "https://youtube.com/watch?v=video-a",
        "https://youtube.com/watch?v=video-b",
    ));
    assert!(!logical_media_ids_match(
        "C:/Alice/Movies/episode.mkv",
        "D:/Bob/Downloads/episode.mkv",
    ));
    assert!(logical_media_ids_match(
        "sha256:stable-logical-id",
        "sha256:stable-logical-id",
    ));
}

#[test]
fn published_logical_identity_is_private_and_stable_across_peer_paths_and_youtube_forms() {
    let alice = LocalFileUpdate::new("episode.mkv")
        .with_size_bytes(42_000)
        .with_path("C:/Alice/episode.mkv");
    let bob = LocalFileUpdate::new("episode.mkv")
        .with_size_bytes(42_000)
        .with_duration_seconds(1_800.0)
        .with_path("D:/Bob/Videos/episode.mkv");
    assert_eq!(
        logical_media_id_for_local_file_update(&alice),
        logical_media_id_for_local_file_update(&bob)
    );

    let watch = LocalFileUpdate::new("https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=share");
    let short = LocalFileUpdate::new("https://youtu.be/dQw4w9WgXcQ?t=12");
    let watch_id = logical_media_id_for_local_file_update(&watch);
    assert_eq!(watch_id, logical_media_id_for_local_file_update(&short));
    assert!(!watch_id.as_str().contains("dQw4w9WgXcQ"));
    assert!(!watch_id.as_str().contains("youtube"));
}

#[test]
fn controller_media_prepare_emits_start_and_room_buffering_policy_once() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Quorum),
        quorum_percent: 60,
        timeout_seconds: 12.0,
        timeout_action: PlaybackBarrierTimeoutAction::Continue,
    });
    runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
        policy: RoomBufferingPolicy::Quorum,
        quorum_percent: 70,
        maximum_pause_seconds: 20.0,
        ..PlaybackBarrierRoomBufferingConfig::default()
    });

    let logical_id = LogicalMediaId::new("media-sha256:opaque-id").unwrap();
    let initial =
        runtime.prepare_playback_media(logical_id.clone(), MediaTransportKind::NetworkVod, 100.0);
    let refreshed = runtime.prepare_playback_media_with_intent(
        logical_id,
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        101.0,
    );

    assert!(initial.logical_media_changed);
    assert!(!refreshed.logical_media_changed);
    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("media preparation should emit a reliable Set");
    };
    let extension = set
        .set
        .playback_barrier_v1()
        .expect("extension should decode")
        .expect("extension should be present");
    let prepare = extension.prepare.expect("start prepare should be present");
    assert_eq!(prepare.logical_media_id, "media-sha256:opaque-id");
    assert_eq!(prepare.policy, PlaybackBarrierPolicy::Quorum);
    assert_eq!(prepare.media_generation, 0);
    assert!(prepare.request_nonce > 0);
    assert_eq!(prepare.load_intent, MediaLoadIntent::NewPlayback);
    assert_eq!(prepare.quorum_percent, Some(60));
    assert_eq!(prepare.timeout_ms, Some(12_000));
    let buffering = extension
        .buffering_policy
        .expect("ongoing buffering policy should be present");
    assert_eq!(buffering.media_generation, prepare.media_generation);
    assert_eq!(buffering.request_nonce, prepare.request_nonce);
    assert_eq!(buffering.load_intent, prepare.load_intent);
    assert_eq!(buffering.policy, RoomBufferingPolicy::Quorum);
    assert_eq!(buffering.quorum_percent, Some(70));
    assert_eq!(buffering.max_pause_ms, Some(20_000));
}

#[test]
fn default_media_prepare_omits_the_start_barrier() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );

    runtime.prepare_playback_media(
        LogicalMediaId::new("media-sha256:immediate-default").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("media preparation should emit a reliable Set");
    };
    let extension = set
        .set
        .playback_barrier_v1()
        .expect("extension should decode")
        .expect("extension should be present");
    assert!(
        extension.prepare.is_none(),
        "the immediate default must not opt in to a coordinated start"
    );
    assert!(
        extension.buffering_policy.is_some(),
        "ongoing room-buffering policy remains independent of start coordination"
    );
}

#[test]
fn client_runtime_debug_redacts_playback_request_and_logical_media_identities() {
    const LOGICAL_MEDIA_MARKER: &str = "private-runtime-logical-media-canary";

    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new(LOGICAL_MEDIA_MARKER).unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request) = &runtime.control().outbound_messages()[0] else {
        panic!("coordination request should use Set");
    };
    let request_id = request
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .and_then(|extension| extension.prepare)
        .and_then(|prepare| prepare.request_id)
        .expect("current clients should attach an opaque request identity");

    let debug = format!("{runtime:?}");
    assert!(!debug.contains(LOGICAL_MEDIA_MARKER));
    assert!(!debug.contains(&request_id));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn startup_media_waits_for_controlled_room_auth_then_emits_once() {
    let mut runtime = ClientRuntime::new(
        controlled_barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
        policy: RoomBufferingPolicy::PauseController,
        ..PlaybackBarrierRoomBufferingConfig::default()
    });

    runtime.prepare_playback_media(
        LogicalMediaId::new("startup-controlled-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "media coordination must wait while controller authentication is pending"
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
        )
        .expect("controller authentication should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("authenticated coordination should emit");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("repeated notification pump should be idempotent");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("authenticated coordination should use a Set envelope");
    };
    let extension = set
        .set
        .playback_barrier_v1()
        .expect("barrier extension should decode")
        .expect("barrier extension should be present");
    assert!(extension.prepare.is_some());
    assert_eq!(
        extension
            .buffering_policy
            .as_ref()
            .map(|policy| policy.policy),
        Some(RoomBufferingPolicy::PauseController)
    );
}

#[test]
fn failed_controlled_room_auth_does_not_emit_media_coordination() {
    let mut runtime = ClientRuntime::new(
        controlled_barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("startup-controlled-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
        )
        .expect("controller authentication failure should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("failed auth notification should dispatch");

    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn transport_write_does_not_acknowledge_start_and_reconnect_queries_server() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.set_playback_barrier_room_buffering_config(PlaybackBarrierRoomBufferingConfig {
        policy: RoomBufferingPolicy::PauseAnyEligible,
        ..PlaybackBarrierRoomBufferingConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("reconnect-current-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(initial_set) = &runtime.control().outbound_messages()[0] else {
        panic!("initial coordination should use a Set envelope");
    };
    let initial_prepare = initial_set
        .set
        .playback_barrier_v1()
        .expect("initial extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("initial request should start a barrier");
    let initial_nonce = initial_prepare.request_nonce;
    let request_id = initial_prepare
        .request_id
        .expect("new clients attach a stable operation id");
    runtime.flush_queued_protocol_messages();
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_some()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_none()
    );

    runtime.begin_protocol_connection_generation();
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("uncertain start should query after reconnect");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("recovery query should be emitted exactly once");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
        panic!("recovery query should use a Set envelope");
    };
    let extension = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .expect("recovery extension should be present");
    assert!(extension.prepare.is_none());
    assert!(extension.buffering_policy.is_none());
    let recovery = extension
        .recovery
        .expect("recovery query should be present");
    assert_eq!(recovery.request_id, request_id);
    assert_eq!(recovery.original_request_nonce, initial_nonce);
    assert!(recovery.recovery_nonce > initial_nonce);
    assert_eq!(recovery.disposition, None);
}

#[test]
fn absent_recovery_result_rebuilds_exactly_one_current_media_start() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("undelivered-current-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(initial_set) = &runtime.control().outbound_messages()[0] else {
        panic!("initial coordination should use a Set envelope");
    };
    let initial_prepare = initial_set
        .set
        .playback_barrier_v1()
        .expect("initial extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("initial request should start a barrier");
    let initial_nonce = initial_prepare.request_nonce;
    let request_id = initial_prepare
        .request_id
        .expect("request id should be present");

    runtime.begin_protocol_connection_generation();
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "the old serialized request must be dropped on reconnect"
    );
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("the current semantic intent should query the server");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
        panic!("recovery query should use a Set envelope");
    };
    let recovery = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .and_then(|extension| extension.recovery)
        .expect("recovery query should be present");
    let recovery_nonce = recovery.recovery_nonce;
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_recovery(
                    PlaybackBarrierRecoveryPayload::result(
                        request_id.clone(),
                        initial_nonce,
                        recovery_nonce,
                        "undelivered-current-media",
                        PlaybackBarrierRecoveryDisposition::Absent,
                    ),
                ),
            ),
        ))
        .expect("explicit absence should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("absence should authorize one fresh start");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("fresh start should not duplicate");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(rebuilt_set) = &runtime.control().outbound_messages()[0] else {
        panic!("rebuilt coordination should use a Set envelope");
    };
    let extension = rebuilt_set
        .set
        .playback_barrier_v1()
        .expect("rebuilt extension should decode")
        .expect("rebuilt extension should be present");
    let prepare = extension
        .prepare
        .expect("absence must rebuild the start intent");
    assert_eq!(prepare.load_intent, MediaLoadIntent::NewPlayback);
    assert_eq!(prepare.request_nonce, initial_nonce);
    assert_eq!(prepare.request_id.as_deref(), Some(request_id.as_str()));
    assert_eq!(
        extension
            .buffering_policy
            .as_ref()
            .map(|policy| policy.request_nonce),
        Some(prepare.request_nonce)
    );
}

#[test]
fn matching_canonical_response_is_the_application_ack_boundary() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("canonical-ack-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone() else {
        panic!("request should use Set");
    };
    let request = request_set
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .expect("request extension should exist");
    runtime.flush_queued_protocol_messages();
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_some()
    );

    let mut canonical_prepare = request.prepare.expect("start should be requested");
    canonical_prepare.media_generation = 7;
    let mut canonical_policy = request
        .buffering_policy
        .expect("buffering policy should be requested");
    canonical_policy.media_generation = 7;
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(canonical_prepare)
                    .with_buffering_policy(canonical_policy),
            ),
        ))
        .expect("canonical response should apply");

    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_some()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_none()
    );
}

#[test]
fn matching_retry_later_retains_exact_operation_and_emits_once_when_due() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("retry-later-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(initial_set) = runtime.control().outbound_messages()[0].clone() else {
        panic!("initial request should use Set");
    };
    let initial = initial_set
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("initial request should contain prepare");
    let request_id = initial.request_id.expect("request id should be present");
    let request_nonce = initial.request_nonce;

    runtime
        .session_mut()
        .apply_protocol_message_at(
            ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_request_result(
                    sorotte_protocol::PlaybackBarrierRequestResultPayload::retry_later(
                        request_id.clone(),
                        request_nonce,
                        1_000,
                    ),
                ),
            )),
            10.0,
        )
        .expect("retry result should apply without terminating the session");

    assert!(runtime.session().is_active());
    assert!(runtime.control().outbound_messages().is_empty());
    assert!(
        runtime
            .playback_coordination
            .barrier
            .initiated_barrier
            .is_none()
    );
    let pending = runtime
        .playback_coordination
        .barrier
        .pending_media_coordination
        .as_ref()
        .expect("semantic media intent must remain pending");
    assert_eq!(pending.request_id, request_id);
    assert_eq!(pending.retry_request_nonce, Some(request_nonce));
    assert_eq!(pending.retry_attempts, 1);
    assert_eq!(
        runtime.pending_playback_barrier_retry_delay_at(10.0),
        Some(1.0)
    );

    runtime
        .run_pending_playback_barrier_retry_at(10.999)
        .expect("early retry pump should be inert");
    assert!(runtime.control().outbound_messages().is_empty());
    runtime
        .run_pending_playback_barrier_retry_at(11.0)
        .expect("due retry should emit");
    runtime
        .run_pending_playback_barrier_retry_at(12.0)
        .expect("repeated retry pump should be idempotent");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(retry_set) = &runtime.control().outbound_messages()[0] else {
        panic!("retry should use Set");
    };
    let retry = retry_set
        .set
        .playback_barrier_v1()
        .expect("retry extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("retry should retain the start request");
    assert_eq!(retry.request_id.as_deref(), Some(request_id.as_str()));
    assert_eq!(retry.request_nonce, request_nonce);
    assert_eq!(retry.logical_media_id, "retry-later-media");
}

#[test]
fn mismatched_retry_later_result_cannot_rearm_current_operation() {
    for (request_id_matches, request_nonce_matches) in
        [(false, true), (true, false), (false, false)]
    {
        let mut runtime = ClientRuntime::new(
            barrier_session(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        runtime.prepare_playback_media(
            LogicalMediaId::new("mismatched-retry-media").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let ProtocolMessage::Set(initial_set) = runtime.control().outbound_messages()[0].clone()
        else {
            panic!("initial request should use Set");
        };
        let initial = initial_set
            .set
            .playback_barrier_v1()
            .expect("request extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("initial request should contain prepare");
        let request_id = initial.request_id.expect("request id should be present");
        let request_nonce = initial.request_nonce;
        let response_id = if request_id_matches {
            request_id.clone()
        } else {
            "another-operation".to_owned()
        };
        let response_nonce = if request_nonce_matches {
            request_nonce
        } else {
            request_nonce.saturating_add(1)
        };

        runtime
            .session_mut()
            .apply_protocol_message_at(
                ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
                    PlaybackBarrierSetExtension::new().with_request_result(
                        sorotte_protocol::PlaybackBarrierRequestResultPayload::retry_later(
                            response_id,
                            response_nonce,
                            1_000,
                        ),
                    ),
                )),
                10.0,
            )
            .expect("mismatched result is syntactically valid");

        assert!(runtime.session().is_active());
        assert_eq!(runtime.control().outbound_messages().len(), 1);
        assert!(
            runtime
                .playback_coordination
                .barrier
                .initiated_barrier
                .is_some()
        );
        assert_eq!(runtime.pending_playback_barrier_retry_delay_at(10.0), None);
        let pending = runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .as_ref()
            .expect("mismatched result must preserve the original pending intent");
        assert_eq!(pending.request_id, request_id);
        assert_eq!(pending.retry_request_nonce, None);
        assert_eq!(pending.retry_attempts, 0);
    }
}

#[test]
fn repeated_retry_later_results_apply_capped_exponential_backoff() {
    assert_eq!(playback_barrier_retry_delay_seconds(1_000, 1), 1.0);
    assert_eq!(playback_barrier_retry_delay_seconds(1_000, 2), 2.0);
    assert_eq!(playback_barrier_retry_delay_seconds(1_000, 6), 30.0);
    assert_eq!(playback_barrier_retry_delay_seconds(u64::MAX, 1), 30.0);
    assert_eq!(playback_barrier_retry_delay_seconds(0, 1), 0.1);
}

#[test]
fn terminal_accepted_reconnect_recovers_before_emitting_any_fresh_request() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("terminal-recovery-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone() else {
        panic!("request should use Set");
    };
    let request = request_set
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .expect("request extension should exist");
    let mut canonical_prepare = request.prepare.expect("start should be requested");
    canonical_prepare.media_generation = 7;
    let mut canonical_policy = request
        .buffering_policy
        .expect("buffering policy should be requested");
    canonical_policy.media_generation = 7;
    let request_id = canonical_prepare
        .request_id
        .clone()
        .expect("request id should be present");
    let request_nonce = canonical_prepare.request_nonce;
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(canonical_prepare.clone())
                    .with_status(barrier_status(7, None, PlaybackBarrierPhase::Degraded))
                    .with_buffering_policy(canonical_policy.clone()),
            ),
        ))
        .expect("terminal canonical lifecycle should apply");
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier_terminal
    );

    runtime.begin_protocol_connection_generation();
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("terminal lifecycle should query recovery first");
    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
        panic!("recovery should use Set");
    };
    let recovery = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .expect("recovery extension should exist");
    assert!(recovery.prepare.is_none());
    assert!(recovery.buffering_policy.is_none());
    let recovery = recovery.recovery.expect("recovery query should be present");
    assert_eq!(recovery.request_id, request_id);
    assert_eq!(recovery.original_request_nonce, request_nonce);
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(canonical_prepare)
                    .with_status(barrier_status(7, None, PlaybackBarrierPhase::Degraded))
                    .with_buffering_policy(canonical_policy)
                    .with_recovery(
                        PlaybackBarrierRecoveryPayload::result(
                            request_id,
                            request_nonce,
                            recovery.recovery_nonce,
                            "terminal-recovery-media",
                            PlaybackBarrierRecoveryDisposition::Recovered,
                        )
                        .with_media_generation(7),
                    ),
            ),
        ))
        .expect("recovered terminal lifecycle should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("recovered terminal lifecycle should not emit fresh intent");
    assert!(runtime.control().outbound_messages().is_empty());
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_some()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_barrier_recovery
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier_terminal
    );
}

#[test]
fn absent_terminal_recovery_emits_only_one_fresh_policy_refresh() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("absent-terminal-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone() else {
        panic!("request should use Set");
    };
    let request = request_set
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .expect("request extension should exist");
    let mut canonical_prepare = request.prepare.expect("start should be requested");
    canonical_prepare.media_generation = 11;
    let mut canonical_policy = request
        .buffering_policy
        .expect("buffering policy should be requested");
    canonical_policy.media_generation = 11;
    let request_id = canonical_prepare
        .request_id
        .clone()
        .expect("request id should be present");
    let request_nonce = canonical_prepare.request_nonce;
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(canonical_prepare)
                    .with_status(barrier_status(11, None, PlaybackBarrierPhase::Degraded))
                    .with_buffering_policy(canonical_policy),
            ),
        ))
        .expect("terminal canonical lifecycle should apply");
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier_terminal
    );

    runtime.begin_protocol_connection_generation();
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("terminal lifecycle should query recovery");
    let ProtocolMessage::Set(recovery_set) = &runtime.control().outbound_messages()[0] else {
        panic!("recovery should use Set");
    };
    let recovery = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .and_then(|extension| extension.recovery)
        .expect("recovery query should be present");
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_recovery(
                    PlaybackBarrierRecoveryPayload::result(
                        request_id,
                        request_nonce,
                        recovery.recovery_nonce,
                        "absent-terminal-media",
                        PlaybackBarrierRecoveryDisposition::Absent,
                    ),
                ),
            ),
        ))
        .expect("explicit terminal absence should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("terminal absence should emit a policy refresh");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("policy refresh should remain exactly once");

    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(refresh_set) = &runtime.control().outbound_messages()[0] else {
        panic!("policy refresh should use Set");
    };
    let refresh = refresh_set
        .set
        .playback_barrier_v1()
        .expect("policy refresh extension should decode")
        .expect("policy refresh extension should exist");
    assert!(refresh.prepare.is_none());
    assert!(refresh.recovery.is_none());
    let policy = refresh
        .buffering_policy
        .expect("policy refresh should be present");
    assert_eq!(policy.media_generation, 0);
    assert_eq!(policy.load_intent, MediaLoadIntent::TransportRefresh);
    assert!(policy.request_nonce > recovery.recovery_nonce);
}

#[test]
fn mismatched_operation_id_cannot_acknowledge_same_nonce() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("nonce-collision-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request_set) = runtime.control().outbound_messages()[0].clone() else {
        panic!("request should use Set");
    };
    let mut prepare = request_set
        .set
        .playback_barrier_v1()
        .expect("request extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("prepare should exist");
    prepare.media_generation = 9;
    prepare.request_id = Some("another-controller-operation".to_owned());
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new()
                .with_playback_barrier_v1(PlaybackBarrierSetExtension::new().with_prepare(prepare)),
        ))
        .expect("peer canonical response should still apply to the session");

    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_some()
    );
}

#[test]
fn matching_identity_on_rejected_zero_generation_echo_is_not_an_application_ack() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("invalid-canonical-echo").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(request) = runtime.control().outbound_messages()[0].clone() else {
        panic!("request should use Set");
    };
    runtime.flush_queued_protocol_messages();
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::Set(request))
        .expect("request-shaped echo is syntactically valid");

    assert!(runtime.session().playback_barrier_prepare().is_none());
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_some()
    );
}

#[test]
fn queued_room_a_start_is_discarded_atomically_on_room_switch() {
    let mut runtime = ClientRuntime::new(
        barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("room-switch-queued-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .initiated_barrier
            .is_some()
    );

    assert!(
        runtime
            .run_set_room("room2")
            .expect("room switch should queue")
    );
    assert!(runtime.control().outbound_messages().iter().all(|message| {
        let ProtocolMessage::Set(set) = message else {
            return true;
        };
        set.set
            .playback_barrier_v1()
            .expect("extension should decode")
            .is_none()
    }));
    assert!(
        runtime
            .playback_coordination
            .barrier
            .initiated_barrier
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .accepted_barrier
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_barrier_recovery
            .is_none()
    );
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_none()
    );

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("authoritative destination room should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("room B should not resurrect the cancelled intent");
    runtime.begin_protocol_connection_generation();
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("reconnect should not resurrect room A");
    assert!(runtime.control().outbound_messages().iter().all(|message| {
        let ProtocolMessage::Set(set) = message else {
            return true;
        };
        set.set
            .playback_barrier_v1()
            .expect("extension should decode")
            .is_none()
    }));
}

#[test]
fn new_controlled_room_reidentify_preserves_unserialized_pre_auth_media() {
    let mut runtime = ClientRuntime::new(
        controlled_barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("new-controlled-room-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    assert!(runtime.control().outbound_messages().is_empty());

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"newControlledRoom":{"roomName":"+other:ZYXWVU654321","password":"AB-123-456"}}}"#,
        )
        .expect("new controlled room should apply");
    runtime
        .run_controller_reidentify_if_needed()
        .expect("reidentify actions should dispatch");
    assert!(
        runtime
            .playback_coordination
            .barrier
            .pending_media_coordination
            .is_some()
    );

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"+other:ZYXWVU654321","success":true}}}"#,
        )
        .expect("destination authentication should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("authenticated destination should emit media coordination");

    let barrier_requests = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter(|message| {
            let ProtocolMessage::Set(set) = message else {
                return false;
            };
            set.set
                .playback_barrier_v1()
                .ok()
                .flatten()
                .is_some_and(|extension| extension.prepare.is_some())
        })
        .count();
    assert_eq!(barrier_requests, 1);
}

#[test]
fn auth_delayed_media_intent_rebinds_to_destination_controlled_room() {
    let mut runtime = ClientRuntime::new(
        controlled_barrier_session(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("room-switch-current-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    assert!(runtime.control().outbound_messages().is_empty());

    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"room":{"name":"+other:ZYXWVU654321"}}}"#)
        .expect("authoritative room switch should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("unauthenticated destination must not emit");
    assert!(runtime.control().outbound_messages().is_empty());

    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"controllerAuth":{"user":"alice","room":"+other:ZYXWVU654321","success":true}}}"#,
        )
        .expect("destination controller authentication should apply");
    runtime
        .run_controller_auth_notifications_if_needed()
        .expect("current media should coordinate in the authenticated destination");
    assert_eq!(runtime.control().outbound_messages().len(), 1);
    let ProtocolMessage::Set(set) = &runtime.control().outbound_messages()[0] else {
        panic!("destination coordination should use a Set envelope");
    };
    assert!(
        set.set
            .playback_barrier_v1()
            .expect("destination extension should decode")
            .and_then(|extension| extension.prepare)
            .is_some()
    );
}

#[test]
fn explicit_replay_can_supersede_same_logical_active_barrier() {
    let logical_id = "active-replay-media";
    let mut session = barrier_session();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(7, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(3),
            )
            .with_status(PlaybackBarrierStatusPayload {
                media_generation: 7,
                state_revision: None,
                phase: PlaybackBarrierPhase::Preparing,
                policy: PlaybackBarrierPolicy::Controller,
                quorum: None,
                deadline: 20.0,
                participants: BTreeMap::new(),
                excluded_legacy_clients: BTreeSet::new(),
            }),
    );
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.set_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    coordination.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let plan = coordination.prepare_media_with_intent(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::Replay,
        10.0,
    );

    let request = coordination
        .playback_barrier_set_for_new_media(&plan, &session, 10.0)
        .expect("explicit replay should not be mistaken for peer participation");
    assert_eq!(
        request.extension.prepare.map(|prepare| prepare.load_intent),
        Some(MediaLoadIntent::Replay)
    );
}

#[test]
fn fresh_controller_infers_replay_from_retained_terminal_logical_identity() {
    let logical_id = "media-sha256:opaque-id";
    let mut session = barrier_session();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(9, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(4),
            )
            .with_commit(CommitStartPayload::new(9, 3, 0.0, 50.0, 55.0))
            .with_status(PlaybackBarrierStatusPayload {
                media_generation: 9,
                state_revision: Some(3),
                phase: PlaybackBarrierPhase::Complete,
                policy: PlaybackBarrierPolicy::Controller,
                quorum: None,
                deadline: 55.0,
                participants: BTreeMap::new(),
                excluded_legacy_clients: BTreeSet::new(),
            }),
    );

    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.set_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    let plan = coordination.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        60.0,
    );
    assert_eq!(plan.load_intent, MediaLoadIntent::NewPlayback);

    let extension = coordination
        .playback_barrier_set_for_new_media(&plan, &session, 60.0)
        .expect("fresh controller should publish an explicit replay request");
    assert_eq!(
        extension
            .extension
            .prepare
            .map(|prepare| prepare.load_intent),
        Some(MediaLoadIntent::Replay)
    );
    assert_eq!(
        extension
            .extension
            .buffering_policy
            .map(|policy| policy.load_intent),
        Some(MediaLoadIntent::Replay)
    );
}

#[test]
fn legacy_server_never_receives_sorotte_barrier_control() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{}}}"#,
        )
        .unwrap();
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    runtime.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(PlaybackBarrierPolicy::AllEligible),
        ..PlaybackBarrierStartConfig::default()
    });
    runtime.prepare_playback_media(
        LogicalMediaId::new("media-sha256:opaque-id").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn ongoing_buffering_reports_only_transport_state_transitions() {
    let logical_id = "media-sha256:opaque-id";
    let mut session = barrier_session();
    let buffering_policy =
        RoomBufferingPolicyPayload::new(10, RoomBufferingPolicy::PauseAnyEligible)
            .with_debounce_ms(750)
            .with_resume_hysteresis_ms(1_500)
            .with_max_pause_ms(30_000);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(10, logical_id, 0.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(10),
            )
            .with_status(PlaybackBarrierStatusPayload {
                media_generation: 10,
                state_revision: None,
                phase: PlaybackBarrierPhase::Preparing,
                policy: PlaybackBarrierPolicy::Controller,
                quorum: None,
                deadline: 120.0,
                participants: BTreeMap::new(),
                excluded_legacy_clients: BTreeSet::new(),
            })
            .with_buffering_policy(buffering_policy.clone()),
    );
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    runtime.control.drain_outbound_messages();

    runtime.observe_external_player_transport(
        transport(1, 1.0, PlayerTransportPhase::Rebuffering, 4.0),
        101.0,
    );
    let first = runtime
        .control
        .drain_outbound_message_lines()
        .expect("buffering report should encode");
    assert!(
        first
            .iter()
            .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":true"))
    );

    runtime.observe_external_player_transport(
        transport(1, 2.0, PlayerTransportPhase::Rebuffering, 4.0),
        102.0,
    );
    assert!(runtime.control().outbound_messages().is_empty());

    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_buffering_policy(buffering_policy.clone())
                    .with_buffering_status(RoomBufferingStatusPayload {
                        config: buffering_policy,
                        phase: RoomBufferingPhase::Monitoring,
                        eligible_clients: 2,
                        required_buffering_clients: 1,
                        buffering_clients: BTreeSet::new(),
                        pause_deadline: None,
                    }),
            ),
        ))
        .expect("authoritative same-policy snapshot should apply");
    runtime.reconcile_external_player_playback(102.1);
    let rearmed = runtime
        .control
        .drain_outbound_message_lines()
        .expect("the snapshot should rearm one current-state report");
    assert!(
        rearmed
            .iter()
            .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":true"))
    );
    runtime.reconcile_external_player_playback(102.2);
    assert!(
        runtime.control().outbound_messages().is_empty(),
        "the snapshot epoch must not cause unchanged per-pump report spam"
    );

    runtime.observe_external_player_transport(
        transport(1, 3.0, PlayerTransportPhase::Playing, 4.5),
        103.0,
    );
    let recovered = runtime
        .control
        .drain_outbound_message_lines()
        .expect("recovery report should encode");
    assert!(
        recovered
            .iter()
            .any(|line| line.contains("\"transport\"") && line.contains("\"buffering\":false"))
    );
}

#[test]
fn server_enforced_prepare_timeout_action_is_captured_once_for_controller_ui() {
    let mut session = barrier_session();
    let mut participants = BTreeMap::new();
    participants.insert(
        "alice-client".to_owned(),
        PlaybackBarrierParticipantStatus {
            phase: PlaybackBarrierParticipantPhase::PrepareTimedOut,
            readiness: None,
            observed_position: None,
            degraded_reason: None,
        },
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    22,
                    "media-sha256:opaque-id",
                    0.0,
                    PlaybackBarrierPolicy::AllEligible,
                )
                .with_request_nonce(22)
                .with_request_id("timeout-request")
                .with_timeout_action(PlaybackBarrierTimeoutAction::RemainPaused),
            )
            .with_status(PlaybackBarrierStatusPayload {
                media_generation: 22,
                state_revision: None,
                phase: PlaybackBarrierPhase::AwaitingDecision,
                policy: PlaybackBarrierPolicy::AllEligible,
                quorum: None,
                deadline: 100.0,
                participants,
                excluded_legacy_clients: BTreeSet::new(),
            }),
    );
    let mut coordination = RuntimePlaybackCoordination::default();
    let plan = coordination.prepare_media(
        LogicalMediaId::new("media-sha256:opaque-id").unwrap(),
        MediaTransportKind::NetworkVod,
        90.0,
    );
    coordination.barrier.initiated_barrier = Some(PlaybackBarrierOperation {
        local_media_generation: plan.media_generation,
        load_intent: MediaLoadIntent::NewPlayback,
        include_start_barrier: true,
        request_id: "timeout-request".to_owned(),
        request_nonce: 22,
        logical_media_id: "media-sha256:opaque-id".to_owned(),
        room: "room1".to_owned(),
    });
    coordination.capture_barrier_timeout_action(&session);
    assert_eq!(
        coordination.barrier.pending_barrier_timeout_action.take(),
        Some(PlaybackBarrierTimeoutAction::RemainPaused)
    );
    coordination.capture_barrier_timeout_action(&session);
    assert_eq!(coordination.barrier.pending_barrier_timeout_action, None);
}

#[test]
fn refreshed_adapter_generation_stays_bound_to_stable_logical_generation() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let logical_id = LogicalMediaId::new("plex://server/item/42").unwrap();
    let initial = runtime.prepare_media(logical_id.clone(), MediaTransportKind::NetworkVod, 0.0);
    runtime.observe_transport(transport(10, 1.0, PlayerTransportPhase::Playing, 1.0), 1.0);
    assert_eq!(
        runtime.logical_generation_for_adapter_generation(10),
        Some(initial.media_generation)
    );

    let refreshed = runtime.prepare_media(logical_id, MediaTransportKind::NetworkVod, 2.0);
    assert_eq!(
        runtime.logical_generation_for_adapter_generation(10),
        None,
        "a mapping from the previous load attempt must not identify the refreshed transport"
    );
    runtime.observe_transport(
        transport(11, 3.0, PlayerTransportPhase::ReadyPaused, 1.0),
        3.0,
    );

    assert_eq!(refreshed.media_generation, initial.media_generation);
    assert_eq!(refreshed.load_attempt, 2);
    assert!(runtime.snapshot().transport_telemetry_observed);
    assert_eq!(
        runtime.snapshot().media_generation,
        Some(initial.media_generation)
    );
    assert_eq!(
        runtime.logical_generation_for_adapter_generation(11),
        Some(initial.media_generation)
    );
    assert_eq!(
        runtime.adapter_generation_bindings.get(&11),
        Some(&LocalTransportGeneration {
            logical_generation: initial.media_generation,
            load_attempt: refreshed.load_attempt,
            adapter_generation: 11,
        })
    );
    assert!(
        !runtime.adapter_generation_bindings.contains_key(&10),
        "refreshing the same logical source must invalidate the prior load attempt"
    );
}

#[test]
fn delayed_previous_load_attempt_cannot_replace_current_transport_observation() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let logical_id = LogicalMediaId::new("plex://server/item/42").unwrap();
    let initial = runtime.prepare_media(logical_id.clone(), MediaTransportKind::NetworkVod, 0.0);
    runtime.observe_transport(transport(10, 1.0, PlayerTransportPhase::Playing, 8.0), 1.0);

    let refreshed = runtime.prepare_media(logical_id, MediaTransportKind::NetworkVod, 2.0);
    assert_eq!(refreshed.media_generation, initial.media_generation);
    assert_eq!(refreshed.load_attempt, initial.load_attempt + 1);

    let stale_before_new_binding =
        runtime.observe_transport(transport(10, 3.0, PlayerTransportPhase::Failed, 99.0), 3.0);
    assert!(stale_before_new_binding.is_empty());
    assert!(runtime.latest_observation.is_none());
    assert!(!runtime.snapshot().transport_telemetry_observed);

    runtime.observe_transport(
        transport(11, 4.0, PlayerTransportPhase::ReadyPaused, 12.0),
        4.0,
    );
    let accepted = runtime.latest_observation.clone().unwrap();
    assert_eq!(accepted.phase, Some(PlayerTransportPhase::ReadyPaused));
    assert_eq!(accepted.position_seconds, Some(12.0));

    let stale_after_new_binding =
        runtime.observe_transport(transport(10, 5.0, PlayerTransportPhase::Ended, 100.0), 5.0);
    assert!(stale_after_new_binding.is_empty());
    assert_eq!(runtime.latest_observation, Some(accepted));
}

#[test]
fn older_same_generation_observation_is_ignored_in_runtime_latest_state() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.observe_transport(
        transport(1, 10.0, PlayerTransportPhase::Playing, 20.0),
        10.0,
    );
    let latest = runtime.latest_observation.clone().unwrap();

    runtime.observe_transport(
        transport(1, 9.0, PlayerTransportPhase::Rebuffering, 5.0),
        11.0,
    );

    assert_eq!(runtime.latest_observation, Some(latest));
    assert_eq!(runtime.snapshot().metrics.stale_timestamp_observations, 1);
    assert_eq!(runtime.last_external_now_seconds, Some(10.0));
    assert_eq!(runtime.last_coordinator_now_seconds, Some(10.0));
    assert_eq!(
        runtime.coordinator_now(12.0),
        12.0,
        "a rejected stale sample must not move the external/coordinator clock anchor backwards"
    );
}

#[test]
fn reconnect_correction_waits_through_loading_cache_pause_and_seek() {
    for phase in [
        PlayerTransportPhase::Loading,
        PlayerTransportPhase::Rebuffering,
        PlayerTransportPhase::Seeking,
    ] {
        let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
        runtime
            .player_mut_for_test()
            .transport_updates
            .push_back(paused_transport(1, 1.0, phase, 0.0));

        runtime
            .run_reconnect_state_restore_validation_if_needed_at(10.0)
            .unwrap();

        assert!(
            runtime.player().commands.is_empty(),
            "{phase:?} must suppress reconnect correction commands"
        );
        assert!(
            runtime
                .session()
                .model
                .reconnect
                .state_restore_validation_pending,
            "{phase:?} must remain pending until a safe observation"
        );
    }
}

#[test]
fn extractor_backed_transport_refresh_keeps_reconnect_coordinator_owned_without_a_fresh_sample() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
    runtime.observe_external_player_transport(
        paused_transport(1, 1.0, PlayerTransportPhase::ReadyPaused, 0.0),
        10.0,
    );
    // Extractor-backed and expiring Plex URLs are NetworkVod transport
    // refreshes: their playback URL changes while logical media identity
    // and the coordinator generation remain stable.
    runtime.prepare_playback_media_with_intent(
        LogicalMediaId::new("reconnect-media").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        10.5,
    );
    runtime.player_mut_for_test().commands.clear();

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.5)
        .unwrap();

    assert!(runtime.player().commands.is_empty());
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending,
        "a known telemetry-capable adapter must not fall back to direct correction during reload"
    );
}

#[test]
fn advertised_telemetry_capability_prevents_direct_reconnect_fallback_before_first_sample() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
    runtime.player_mut_for_test().advertises_telemetry = true;

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();

    assert!(runtime.player().commands.is_empty());
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
}

#[test]
fn telemetry_capability_without_a_media_transaction_retains_legacy_reconnect_correction() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":100.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            10.0,
        )
        .unwrap();
    session.model.playback.local_paused = Some(true);
    session.model.playback.local_position = Some(0.0);
    session.model.reconnect.state_restore_validation_pending = true;
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();

    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
    )));
    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending,
        "capability alone cannot select a coordinator with no media identity"
    );
}

#[test]
fn accepted_reconnect_command_needs_matching_transport_observation() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            1.0,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ));

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();
    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
    )));
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
    assert_eq!(
        runtime
            .reconnect_state_restore_correction_metrics()
            .correction_actions_succeeded,
        0
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.5)
        .unwrap();
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending,
        "dispatch acceptance without observation must not complete reconnect validation"
    );

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            2.0,
            PlayerTransportPhase::ReadyPaused,
            100.0,
        ));
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(11.0)
        .unwrap();
    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
    assert_eq!(
        runtime
            .reconnect_state_restore_correction_metrics()
            .correction_actions_succeeded,
        1
    );
}

#[test]
fn reconnect_reconciliation_corrects_self_attributed_room_state() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
    runtime
        .session_mut_for_test()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":100.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            10.0,
        )
        .unwrap();
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            1.0,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ));

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();
    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
    )));
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending,
        "self attribution must suppress ordinary echo replay, not reconnect reconciliation"
    );

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            2.0,
            PlayerTransportPhase::ReadyPaused,
            100.0,
        ));
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(11.0)
        .unwrap();
    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
}

#[test]
fn newer_room_state_supersedes_stale_reconnect_seek() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            1.0,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ));
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();

    runtime
        .session_mut_for_test()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":200.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            10.5,
        )
        .unwrap();
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.5)
        .unwrap();
    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 200.0).abs() < f64::EPSILON
    )));

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            2.0,
            PlayerTransportPhase::ReadyPaused,
            100.0,
        ));
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(11.0)
        .unwrap();
    assert!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending,
        "an observation matching only the superseded seek must not complete validation"
    );

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            3.0,
            PlayerTransportPhase::ReadyPaused,
            200.0,
        ));
    runtime
        .run_reconnect_state_restore_validation_if_needed_at(12.0)
        .unwrap();
    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
}

#[test]
fn reconnect_without_transport_telemetry_retains_legacy_direct_correction() {
    let mut runtime = reconnect_runtime(true, 100.0, true, 0.0);

    runtime
        .run_reconnect_state_restore_validation_if_needed_at(10.0)
        .unwrap();

    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 100.0).abs() < f64::EPSILON
    )));
    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
}

#[test]
fn manual_seek_interrupts_recovery_and_resets_owned_rate_first() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    assert!(runtime.run_seek_to_position(20.0).unwrap());

    assert!(matches!(
        runtime.player().commands.as_slice(),
        [PlayerCommand::SetPlaybackRate(rate), PlayerCommand::SetPosition(position)]
            if (*rate - 1.0).abs() < f64::EPSILON
                && (*position - 20.0).abs() < f64::EPSILON
    ));
    assert!(
        runtime
            .playback_coordination
            .snapshot()
            .recovery_episode
            .is_none()
    );
}

#[test]
fn local_pause_interrupts_recovery_and_resets_owned_rate_first() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    assert!(runtime.run_set_paused(true).unwrap());
    assert!(matches!(
        runtime.player().commands.as_slice(),
        [PlayerCommand::SetPlaybackRate(rate), PlayerCommand::SetPaused(true)]
            if (*rate - 1.0).abs() < f64::EPSILON
    ));
}

#[test]
fn rejected_rate_cleanup_does_not_swallow_the_users_manual_seek() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            reject_rate_commands: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    assert!(runtime.run_seek_to_position(20.0).unwrap());
    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPosition(position) if (*position - 20.0).abs() < f64::EPSILON
    )));
}

#[test]
fn rejected_rate_cleanup_does_not_swallow_the_users_pause() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            reject_rate_commands: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    assert!(runtime.run_set_paused(true).unwrap());
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
    );
}

#[test]
fn disconnect_resets_owned_rate_and_preserves_pause_on_leave() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    runtime.run_disconnect(20.0).unwrap();

    assert!(runtime.player().commands.iter().any(|command| matches!(
        command,
        PlayerCommand::SetPlaybackRate(rate) if (*rate - 1.0).abs() < f64::EPSILON
    )));
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
    );
    assert!(
        runtime
            .playback_coordination
            .snapshot()
            .recovery_episode
            .is_none()
    );
}

#[test]
fn rejected_rate_cleanup_does_not_abort_disconnect() {
    let coordination = coordination_with_owned_catchup_rate();
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    session.model.playback.local_position = Some(11.0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            reject_rate_commands: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination = coordination;

    runtime.run_disconnect(20.0).unwrap();
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
    );
}

#[test]
fn stale_adapter_generation_cannot_seize_current_transport_ownership() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 1.0), 1.0);
    runtime.prepare_media(
        LogicalMediaId::new("episode-2").unwrap(),
        MediaTransportKind::NetworkVod,
        2.0,
    );

    let actions =
        runtime.observe_transport(transport(1, 3.0, PlayerTransportPhase::Playing, 100.0), 3.0);

    assert!(actions.is_empty());
    assert!(!runtime.snapshot().transport_telemetry_observed);
    assert_eq!(runtime.snapshot().metrics.stale_generation_observations, 0);
    assert_eq!(runtime.last_external_now_seconds, Some(1.0));
    assert_eq!(runtime.last_coordinator_now_seconds, Some(1.0));
    assert_eq!(
        runtime.coordinator_now(4.0),
        4.0,
        "an observation from the superseded media generation must not replace clock anchors"
    );
}

#[test]
fn desync_position_projection_uses_the_position_sample_clock_and_rejects_stale_samples() {
    let mut runtime = RuntimePlaybackCoordination::default();
    assert_eq!(
        runtime.projected_local_position_at(100.0, Some(7.0)),
        Some(7.0),
        "legacy adapters without transport timestamps retain point-sample compatibility"
    );
    runtime.prepare_media(
        LogicalMediaId::new("clock-aligned-position").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    runtime.observe_transport(
        transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
        101.0,
    );

    let mut sparse_update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.5)),
    )
    .with_phase(PlayerTransportPhase::Playing)
    .with_logical_pause(false);
    sparse_update.paused_for_cache = Some(false);
    sparse_update.seeking = Some(false);
    sparse_update.core_idle = Some(false);
    runtime.observe_transport(sparse_update, 101.5);

    let projected = runtime
        .projected_local_position_at(101.8, Some(10.0))
        .expect("a fresh playing sample should be projected");
    assert!(
        (projected - 10.8).abs() <= 0.000_001,
        "a newer sparse event must not rewrite the older position field's sample clock"
    );
    assert_eq!(
        runtime.projected_local_position_at(103.1, Some(10.0)),
        None,
        "room synchronization must wait for fresh telemetry instead of aging a stale point sample indefinitely"
    );

    let mut slowed = transport(1, 4.0, PlayerTransportPhase::Playing, 20.0);
    slowed.playback_rate = Some(0.95);
    runtime.observe_transport(slowed, 104.0);
    let projected_slowed = runtime
        .projected_local_position_at(104.5, Some(20.0))
        .expect("fresh slowed playback should remain projectable");
    assert!((projected_slowed - 20.475).abs() <= 0.000_001);

    runtime.observe_transport(
        paused_transport(1, 5.0, PlayerTransportPhase::ReadyPaused, 30.0),
        105.0,
    );
    assert_eq!(
        runtime.projected_local_position_at(120.0, Some(30.0)),
        Some(30.0),
        "a positively paused point sample does not age"
    );

    runtime.observe_transport(
        transport(1, 6.0, PlayerTransportPhase::Rebuffering, 30.0),
        106.0,
    );
    assert_eq!(
        runtime.projected_local_position_at(106.1, Some(30.0)),
        None,
        "room synchronization must not project a cache-stalled sample"
    );
}

#[test]
fn authoritative_transport_rebase_clears_every_absent_sparse_field() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("authoritative-rebase").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let generation = PlayerMediaGeneration::new(1);
    let mut stale = PlayerTransportTelemetryUpdate::new(
        generation,
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.0)),
    )
    .with_phase(PlayerTransportPhase::Seeking)
    .with_position_seconds(45.0)
    .with_logical_pause(true);
    stale.playback_rate = Some(0.95);
    stale.paused_for_cache = Some(true);
    stale.cache_buffering_percent = Some(12.0);
    stale.seeking = Some(true);
    stale.seekable = Some(true);
    stale.timeline_kind = Some(sorotte_player_api::PlayerTimelineKind::SlidingLive);
    stale.core_idle = Some(true);
    stale.playback_restart_sequence = Some(9);
    stale.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
        100.0, 160.0,
    )]);
    stale.known_live_seekable_window =
        Some(sorotte_player_api::PlayerSeekableRange::new(100.0, 160.0));
    stale.buffered_ahead_seconds = Some(60.0);
    stale.input_rate_bytes_per_second = Some(1_000_000);
    runtime.observe_transport(stale, 1.0);

    let replacement = PlayerTransportTelemetryUpdate::new(
        generation,
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(2.0)),
    )
    .with_phase(PlayerTransportPhase::Playing);
    runtime.rebase_transport(replacement, 2.0);

    let latest = runtime
        .latest_observation
        .as_ref()
        .expect("replacement observation should remain current");
    assert_eq!(latest.phase, Some(PlayerTransportPhase::Playing));
    assert_eq!(latest.position_seconds, None);
    assert_eq!(latest.playback_rate, None);
    assert_eq!(latest.logical_pause, None);
    assert_eq!(latest.paused_for_cache, None);
    assert_eq!(latest.seeking, None);
    assert_eq!(latest.seekable, None);
    assert_eq!(latest.timeline_kind, None);
    assert_eq!(latest.seekable_ranges, None);
    assert_eq!(latest.known_live_seekable_window, None);
    assert_eq!(latest.core_idle, None);
    assert_eq!(latest.playback_restart_sequence, None);
    assert_eq!(latest.cache_buffering_percent, None);
    assert_eq!(latest.buffered_ahead_seconds, None);
    assert_eq!(latest.input_rate_bytes_per_second, None);
    assert!(runtime.latest_position_observation.is_none());

    let observed = runtime
        .coordinator
        .observed_transport_for_test()
        .expect("coordinator should accept the replacement");
    assert_eq!(observed.phase, Some(PlayerTransportPhase::Playing));
    assert_eq!(observed.position_seconds, None);
    assert_eq!(observed.playback_rate, None);
    assert_eq!(observed.logical_pause, None);
    assert_eq!(observed.paused_for_cache, Some(false));
    assert_eq!(observed.seeking, Some(false));
    assert_eq!(observed.seekable, None);
    assert_eq!(observed.timeline_kind, None);
    assert_eq!(observed.seekable_ranges, None);
    assert_eq!(observed.known_live_seekable_window, None);
    assert_eq!(observed.core_idle, None);
    assert_eq!(observed.playback_restart_sequence, Some(0));
    assert_eq!(observed.cache_buffering_percent, None);
    assert_eq!(observed.buffered_ahead_seconds, None);
    assert_eq!(runtime.snapshot().metrics.last_buffered_ahead_seconds, None);
    assert_eq!(
        runtime.snapshot().metrics.last_input_rate_bytes_per_second,
        None
    );
}

#[test]
fn sparse_rate_transitions_preserve_piecewise_position_and_actual_sample_age() {
    for playback_rate in [0.5, 0.95, 2.0, 4.0] {
        let mut position_then_speed = RuntimePlaybackCoordination::default();
        position_then_speed.prepare_media(
            LogicalMediaId::new(format!("position-then-speed-{playback_rate}")).unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        position_then_speed.observe_transport(
            transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
            101.0,
        );
        let mut speed = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(2.0)),
        );
        speed.playback_rate = Some(playback_rate);
        position_then_speed.observe_transport(speed, 102.0);

        let projected = position_then_speed
            .projected_local_position_at(102.5, None)
            .expect("a fresh piecewise position should remain projectable");
        let expected = 11.0 + 0.5 * playback_rate;
        assert!(
            (projected - expected).abs() <= 0.000_001,
            "position->speed at {playback_rate}x projected {projected}, expected {expected}"
        );
        assert_eq!(
            position_then_speed.projected_local_position_at(103.1, None),
            None,
            "a rate transition must not refresh the last actual position at {playback_rate}x"
        );

        let mut speed_then_position = RuntimePlaybackCoordination::default();
        speed_then_position.prepare_media(
            LogicalMediaId::new(format!("speed-then-position-{playback_rate}")).unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        let mut initial = transport(1, 0.0, PlayerTransportPhase::Playing, 0.0);
        initial.playback_rate = Some(1.0);
        speed_then_position.observe_transport(initial, 100.0);
        let mut speed = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.0)),
        );
        speed.playback_rate = Some(playback_rate);
        speed_then_position.observe_transport(speed, 101.0);
        let expected_position = 1.0 + playback_rate;
        let mut position = transport(1, 2.0, PlayerTransportPhase::Playing, expected_position);
        position.playback_rate = None;
        speed_then_position.observe_transport(position, 102.0);

        let projected = speed_then_position
            .projected_local_position_at(102.5, None)
            .expect("a position after a sparse rate should remain projectable");
        let expected = expected_position + 0.5 * playback_rate;
        assert!(
            (projected - expected).abs() <= 0.000_001,
            "speed->position at {playback_rate}x projected {projected}, expected {expected}"
        );
    }

    let mut multiple_transitions = RuntimePlaybackCoordination::default();
    multiple_transitions.prepare_media(
        LogicalMediaId::new("multiple-rate-transitions").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    multiple_transitions.observe_transport(
        transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
        101.0,
    );
    for (observed_at_seconds, playback_rate) in [(1.5, 2.0), (2.0, 0.5)] {
        let mut speed = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(
                observed_at_seconds,
            )),
        );
        speed.playback_rate = Some(playback_rate);
        multiple_transitions.observe_transport(speed, 100.0 + observed_at_seconds);
    }
    let projected = multiple_transitions
        .projected_local_position_at(102.5, None)
        .expect("multiple sparse rate segments should remain projectable");
    assert!((projected - 11.75).abs() <= 0.000_001);
}

#[test]
fn first_delayed_transport_sample_preserves_queue_dwell_and_invalid_rates_fail_closed() {
    let mut delayed = RuntimePlaybackCoordination::default();
    delayed.prepare_media(
        LogicalMediaId::new("first-delayed-transport-sample").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    let mut delayed_position = transport(1, 1.0, PlayerTransportPhase::Playing, 10.0);
    delayed_position.observed_at = Some(PlayerObservationTimestamp::from_adapter_observation(
        Duration::from_secs(1),
        Duration::from_secs(5),
    ));
    delayed.observe_transport(delayed_position, 105.0);
    assert_eq!(
        delayed.projected_local_position_at(105.0, None),
        None,
        "the first rich sample must retain its four seconds of queue dwell"
    );

    for invalid_rate in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new(format!("invalid-rate-{invalid_rate}")).unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        runtime.observe_transport(
            transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
            101.0,
        );
        let mut invalid = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
        );
        invalid.playback_rate = Some(invalid_rate);
        runtime.observe_transport(invalid, 102.0);
        assert_eq!(
            runtime.projected_local_position_at(102.1, None),
            None,
            "invalid playback rate {invalid_rate} must make projection unavailable"
        );
        runtime.observe_transport(
            transport(1, 2.2, PlayerTransportPhase::Playing, 11.0),
            102.2,
        );
        assert_eq!(
            runtime.projected_local_position_at(102.3, None),
            None,
            "an absent rate must not silently replace invalid rate {invalid_rate}"
        );
        let mut recovered = transport(1, 2.4, PlayerTransportPhase::Playing, 11.2);
        recovered.playback_rate = Some(1.0);
        runtime.observe_transport(recovered, 102.4);
        assert!(
            runtime.projected_local_position_at(102.5, None).is_some(),
            "a later valid rate should recover projection"
        );
    }
}

#[test]
fn sparse_pause_and_cache_transitions_close_and_resume_the_position_clock() {
    for cache_pause in [false, true] {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new(format!("sparse-motion-transition-{cache_pause}")).unwrap(),
            MediaTransportKind::NetworkVod,
            100.0,
        );
        runtime.observe_transport(
            transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
            101.0,
        );
        let mut stopped = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.5)),
        );
        stopped.phase = Some(if cache_pause {
            PlayerTransportPhase::Rebuffering
        } else {
            PlayerTransportPhase::ReadyPaused
        });
        stopped.logical_pause = Some(!cache_pause);
        stopped.paused_for_cache = Some(cache_pause);
        stopped.core_idle = Some(!cache_pause);
        runtime.observe_transport(stopped, 101.5);
        if !cache_pause {
            assert_eq!(
                runtime.projected_local_position_at(120.0, None),
                Some(10.5),
                "pause edge must retain progress through its own timestamp"
            );
        }

        let mut resumed = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
        );
        resumed.phase = Some(PlayerTransportPhase::Playing);
        resumed.logical_pause = Some(false);
        resumed.paused_for_cache = Some(false);
        resumed.core_idle = Some(false);
        runtime.observe_transport(resumed, 102.0);
        let projected = runtime
            .projected_local_position_at(102.2, None)
            .expect("a short sparse stop should remain projectable after resume");
        assert!(
            (projected - 10.7).abs() <= 0.000_001,
            "projection must exclude the stopped interval for cache_pause={cache_pause}"
        );
    }
}

#[test]
fn stale_playing_sample_cannot_become_a_trusted_sparse_pause_anchor() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("stale-playing-to-sparse-pause").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    runtime.observe_transport(
        transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
        101.0,
    );
    let mut paused = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(4)),
    );
    paused.phase = Some(PlayerTransportPhase::ReadyPaused);
    paused.logical_pause = Some(true);
    paused.paused_for_cache = Some(false);
    paused.seeking = Some(false);
    paused.core_idle = Some(true);

    runtime.observe_transport(paused, 104.0);

    assert_eq!(
        runtime.projected_local_position_at(104.0, None),
        None,
        "a sparse pause must not make a position last sampled three seconds ago authoritative"
    );
    assert_eq!(
        runtime.projected_local_position_at(120.0, None),
        None,
        "the rejected inferred pause anchor must not become permanently trusted"
    );
}

#[test]
fn outbound_state_sync_projects_rich_position_and_omits_blocked_samples() {
    let mut session = barrier_session();
    session.model.playback.local_position = Some(10.0);
    session.model.playback.local_paused = Some(false);
    session
        .apply_protocol_message_at(
            ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(5.0)
                        .with_paused(false)
                        .with_set_by("bob"),
                ),
            ),
            99.0,
        )
        .expect("the projection test should begin after a canonical room baseline");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("outbound-position-clock").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    runtime.playback_coordination.observe_transport(
        transport(1, 1.0, PlayerTransportPhase::Playing, 10.0),
        101.0,
    );

    let outbound_position = runtime
        .outbound_state_sync_position_seconds(101.8, false)
        .expect("fresh rich transport should emit a playback sample");
    assert!(
        (outbound_position - 10.8).abs() <= 0.000_001,
        "the server must receive the local position projected to the State message clock"
    );

    runtime.playback_coordination.observe_transport(
        transport(1, 4.5, PlayerTransportPhase::Playing, 14.5),
        104.5,
    );
    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(5.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            ),
            false,
            100.0,
            105.0,
            100.0,
        )
    );
    let ProtocolMessage::State(delayed_response) = runtime
        .control()
        .outbound_messages()
        .back()
        .expect("the delayed inbound State should receive a response")
    else {
        panic!("the delayed inbound State response should remain a State message");
    };
    let delayed_response_position = delayed_response
        .state
        .playstate
        .as_ref()
        .and_then(|playstate| playstate.position);
    assert!(
        delayed_response_position.is_some_and(|position| (position - 15.0).abs() <= 0.000_001),
        "receipt time remains the inbound clock, but the outbound local sample must be projected to the later response clock: {delayed_response_position:?}"
    );

    runtime.playback_coordination.observe_transport(
        transport(1, 5.0, PlayerTransportPhase::Rebuffering, 15.0),
        105.0,
    );
    assert_eq!(
        runtime.outbound_state_sync_position_seconds(105.1, false),
        None,
        "cache-stalled rich telemetry must produce ping-only State instead of re-anchoring the room"
    );
}

#[test]
fn authorized_play_intent_survives_cache_blocked_position_projection() {
    let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            now_seconds,
        )
        .expect("legacy room Hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            now_seconds,
        )
        .expect("canonical paused state should apply");
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("cache-blocked-local-play").unwrap(),
        MediaTransportKind::LocalFile,
        now_seconds,
    );
    let mut cache_paused = transport(1, 0.0, PlayerTransportPhase::Rebuffering, 0.0);
    cache_paused.logical_pause = Some(true);
    cache_paused.core_idle = Some(true);
    runtime
        .playback_coordination
        .observe_transport(cache_paused, now_seconds);
    runtime.session.model.playback.local_position = Some(0.0);
    runtime.session.model.playback.local_paused = Some(false);

    assert_eq!(
        runtime.outbound_state_sync_position_seconds(now_seconds, false),
        None,
        "cache-paused telemetry must remain blocked from ordinary State publication"
    );
    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let ordinary = runtime.flush_queued_protocol_messages();
    let ProtocolMessage::State(ordinary) = ordinary
        .last()
        .expect("ordinary cache-paused heartbeat should be queued")
    else {
        panic!("ordinary cache-paused heartbeat should remain State");
    };
    assert!(
        ordinary.state.playstate.is_none(),
        "cache telemetry without semantic intent must remain ping-only"
    );

    runtime.stage_external_player_pause_intent(false, now_seconds);
    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let mutation = runtime.flush_queued_protocol_messages();
    let ProtocolMessage::State(mutation) = mutation
        .last()
        .expect("authorized Play heartbeat should be queued")
    else {
        panic!("authorized Play heartbeat should remain State");
    };
    let playstate = mutation
        .state
        .playstate
        .as_ref()
        .expect("authorized Play must not be downgraded to ping-only by cache telemetry");
    assert_eq!(playstate.position, Some(0.0));
    assert_eq!(playstate.paused, Some(false));
    assert_ne!(
        playstate.do_seek,
        Some(true),
        "Play/Pause fallback must never become a seek"
    );
    assert_eq!(playstate.set_by, None);
}

#[test]
fn pause_mutation_position_never_crosses_media_generation() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("legacy room Hello should apply");
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("retired-pause-anchor").unwrap(),
        MediaTransportKind::LocalFile,
        100.0,
    );
    let mut cache_paused = transport(1, 0.0, PlayerTransportPhase::Rebuffering, 7.5);
    cache_paused.logical_pause = Some(true);
    cache_paused.core_idle = Some(true);
    coordination.observe_transport(cache_paused, 100.0);
    coordination.stage_local_pause_intent(false, &session);
    assert_eq!(
        coordination.local_pause_mutation_position_at(100.1, Some(99.0)),
        Some(7.5),
        "the current generation may anchor its authorized command despite cache pause"
    );

    coordination.prepare_media(
        LogicalMediaId::new("replacement-pause-anchor").unwrap(),
        MediaTransportKind::LocalFile,
        100.2,
    );
    session.model.playback.local_position = Some(99.0);
    assert_eq!(
        coordination.local_pause_mutation_position_at(100.3, session.model.playback.local_position),
        None,
        "retired rich telemetry and its legacy mirror must not anchor the replacement media"
    );
    assert_eq!(
        coordination.active_local_pause_state_mutation_intent(&session),
        None,
        "a predecessor command must not survive the replacement media generation"
    );
}

#[test]
fn delayed_inbound_state_does_not_publish_false_local_seek() {
    let mut session = barrier_session();
    session.model.playback.local_position = Some(5.0);
    session.model.playback.local_paused = Some(false);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("delayed-inbound-state").unwrap(),
        MediaTransportKind::NetworkVod,
        99.0,
    );
    runtime
        .playback_coordination
        .observe_transport(transport(1, 1.0, PlayerTransportPhase::Playing, 5.0), 100.0);
    runtime
        .playback_coordination
        .observe_transport(transport(1, 5.5, PlayerTransportPhase::Playing, 9.5), 104.5);

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(5.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            ),
            false,
            100.0,
            105.0,
            100.0,
        )
    );
    let ProtocolMessage::State(response) = runtime
        .control()
        .outbound_messages()
        .back()
        .expect("the delayed inbound State should receive a response")
    else {
        panic!("the delayed inbound State response should remain a State message");
    };
    let response_playstate = response
        .state
        .playstate
        .as_ref()
        .expect("fresh local telemetry should remain present");
    assert!(
        response_playstate
            .position
            .is_some_and(|position| (position - 10.0).abs() <= 0.000_001),
        "local state should be projected to reply time"
    );
    assert_ne!(
        response_playstate.do_seek,
        Some(true),
        "matching room and local positions at receipt time must not become a local seek solely because the GUI processes the inbound State later"
    );
}

#[test]
fn adapter_epoch_reset_accepts_restarted_generation_and_rejects_old_epoch() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        90.0,
    );
    runtime.observe_transport_at_epoch(
        transport(5, 50.0, PlayerTransportPhase::Playing, 5.0),
        95.0,
        0,
    );
    let next_epoch = runtime.reset_adapter_epoch(100.0);
    assert_eq!(next_epoch, 1);

    let stale = runtime.observe_transport_at_epoch(
        transport(6, 51.0, PlayerTransportPhase::Playing, 99.0),
        101.0,
        0,
    );
    assert!(stale.is_empty());
    assert!(!runtime.snapshot().transport_telemetry_observed);

    runtime.observe_transport_at_epoch(
        transport(1, 1.0, PlayerTransportPhase::ReadyPaused, 5.0),
        102.0,
        next_epoch,
    );
    assert!(runtime.snapshot().transport_telemetry_observed);
    assert_eq!(
        runtime
            .latest_observation
            .as_ref()
            .map(|observation| observation.observed_at_seconds),
        Some(102.0)
    );
}

#[test]
fn adapter_epoch_reset_rearms_degraded_seek_for_same_room_state() {
    let mut session = barrier_session();
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":40.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("remote seek should apply");
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.update_desired_from_session(&session, 0.0);

    let initial = runtime.observe_transport_at_epoch(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0),
        0.1,
        0,
    );
    assert!(initial.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));

    runtime.coordinator.tick(1_000.0);
    assert!(matches!(
        runtime.coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(_))
    ));

    let next_epoch = runtime.reset_adapter_epoch(1_001.0);
    assert_eq!(next_epoch, 1);
    assert_eq!(
        runtime.coordinator.last_seek_preparation_terminal_outcome(),
        None,
        "a replacement player must not inherit the old degraded seek hold"
    );
    assert!(
        runtime.coordinator.recovery_episode().is_none(),
        "a replacement player must receive a fresh recovery budget"
    );

    runtime.update_desired_from_session(&session, 1_001.1);
    let replacement = runtime.observe_transport_at_epoch(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0),
        1_001.2,
        next_epoch,
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));
}

#[test]
fn unattributed_room_state_without_peers_does_not_become_coordinator_desire() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":3.0,"paused":true,"doSeek":false}}}"#,
            3.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("unattributed-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );

    assert!(
        coordination
            .update_desired_from_session(&session, 3.0)
            .is_empty()
    );
    assert!(coordination.desired_fingerprint.is_none());
    assert_eq!(coordination.coordinator.desired_revision_pending(), None);
}

#[test]
fn local_pause_and_unpause_echoes_become_desired_without_command_replay() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("local-echo-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.0), 0.0);
    coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.2), 0.2);

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            1.0,
        )
        .unwrap();
    let pause_echo = coordination.update_desired_from_session(&session, 1.0);
    assert!(!has_pause_play_or_seek(&pause_echo));
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused)
    );
    assert!(
        coordination.snapshot().ordinary_correction_blocked,
        "the coordinator owns the local desired revision until telemetry confirms it"
    );
    let paused = coordination.observe_transport(
        paused_transport(1, 1.1, PlayerTransportPhase::ReadyPaused, 1.0),
        1.1,
    );
    assert!(!has_pause_play_or_seek(&paused));
    assert!(!coordination.snapshot().ordinary_correction_blocked);

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            2.0,
        )
        .unwrap();
    let unpause_echo = coordination.update_desired_from_session(&session, 2.0);
    assert!(!has_pause_play_or_seek(&unpause_echo));
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| !desired.paused)
    );
    let playing =
        coordination.observe_transport(transport(1, 2.1, PlayerTransportPhase::Playing, 1.2), 2.1);
    assert!(!has_pause_play_or_seek(&playing));
    assert!(!coordination.snapshot().ordinary_correction_blocked);
}

#[test]
fn local_echo_seek_preparation_deadline_advances_without_command_replay() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":40.0,"paused":true,"doSeek":true,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination
        .coordinator
        .set_config(PlaybackCoordinatorConfig {
            command_timeout_seconds: 1.0,
            seek_preparation_timeout_seconds: 2.0,
            ..PlaybackCoordinatorConfig::default()
        });
    coordination.prepare_media(
        LogicalMediaId::new("local-echo-slow-seek").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );

    coordination.update_desired_from_session(&session, 0.0);
    assert!(coordination.snapshot().seek_preparation.is_some());

    let timed_out = coordination.update_desired_from_session(&session, 2.1);
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert_eq!(
        coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimedOut
        ))
    );
}

#[test]
fn staged_local_unpause_survives_transport_observation_before_canonical_echo() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("staged-local-unpause").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );

    coordination.stage_local_pause_intent(false, &session);
    let staged = coordination.update_desired_from_session_with_replay(&session, 0.1, false);
    assert!(!has_pause_play_or_seek(&staged));
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false)
    );
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| !desired.paused && desired.local_echo)
    );

    let observed_play =
        coordination.observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);
    assert!(
        !observed_play.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )),
        "the pre-echo playing observation must not replay the stale canonical pause"
    );

    let extra_pre_echo_pump = coordination.update_desired_from_session(&session, 0.2);
    assert!(
        !extra_pre_echo_pump.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )),
        "an extra reconciliation pump must retain the staged unpause"
    );
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false)
    );

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.3,
        )
        .unwrap();
    let canonical_echo = coordination.update_desired_from_session(&session, 0.3);
    assert!(!has_pause_play_or_seek(&canonical_echo));
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        None,
        "the matching canonical echo must retire the staged intent"
    );

    let continued_play = coordination
        .observe_transport(transport(1, 0.4, PlayerTransportPhase::Playing, 10.25), 0.4);
    assert!(!has_pause_play_or_seek(&continued_play));
    assert!(
        !coordination.snapshot().ordinary_correction_blocked,
        "advancing playback after the echo must complete coordinator ownership"
    );
}

#[test]
fn attached_local_play_suppresses_seek_preparation_pause_before_canonical_echo() {
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            0.0,
        )
        .expect("legacy Hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .expect("initial canonical pause should apply");
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("attached-local-play-seek-echo").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.reconcile_external_player_playback(0.0);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 0.0),
        0.0,
    );

    let staged = runtime.stage_external_player_pause_intent(false, 0.1);
    assert!(!has_pause_play_or_seek(&staged));
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false),
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"alice"}}}"#,
            0.11,
        )
        .expect("the local seek echo should apply before the Play echo");
    let seek_echo = runtime.reconcile_external_player_playback(0.11);
    assert!(!has_pause_play_or_seek(&seek_echo));
    assert!(
        runtime
            .playback_coordination_snapshot()
            .seek_preparation
            .is_some(),
        "the network seek echo should reproduce the preparation window"
    );

    let observed_play = runtime.observe_external_player_transport(
        transport(1, 0.12, PlayerTransportPhase::Playing, 0.0),
        0.12,
    );
    assert!(
        !observed_play.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )),
        "the in-flight local Play must supersede a preparation pause before dispatch: {observed_play:?}"
    );
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false),
        "suppressing the stale pause must retain the local intent until its canonical echo"
    );
}

#[test]
fn matching_remote_canonical_pause_state_waits_for_player_before_retiring_local_overlay() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("remote-acknowledged-unpause").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );
    coordination.stage_local_pause_intent(false, &session);
    coordination.update_desired_from_session_with_replay(&session, 0.1, false);

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.3,
        )
        .unwrap();
    coordination.update_desired_from_session(&session, 0.3);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false),
        "matching canonical truth must not retire the command while the player still reports the preceding pause"
    );

    coordination.observe_transport(
        transport(1, 0.35, PlayerTransportPhase::Playing, 10.0),
        0.35,
    );
    coordination.update_desired_from_session(&session, 0.35);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        None,
        "canonical and physical confirmation together acknowledge the command even when another user owns the room anchor"
    );

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.4,
        )
        .unwrap();
    coordination.update_desired_from_session(&session, 0.4);
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused && !desired.local_echo),
        "a retired command must not override a later legitimate room pause"
    );
}

#[test]
fn staged_local_unpause_retires_after_repeated_newer_canonical_pauses() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("stale-local-unpause").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );

    coordination.stage_local_pause_intent(false, &session);
    coordination.update_desired_from_session_with_replay(&session, 0.1, false);
    coordination.observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);

    for update_at in [1.0, 6.0] {
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                update_at,
            )
            .unwrap();
        let actions = coordination.update_desired_from_session(&session, update_at);
        assert!(!has_pause_play_or_seek(&actions));
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false),
            "a bounded command/echo window must tolerate delayed canonical delivery"
        );
    }

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            11.0,
        )
        .unwrap();
    let reconciliation = coordination.update_desired_from_session(&session, 11.0);
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    assert!(
        reconciliation.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )),
        "after the bounded echo window the canonical room pause must regain player authority"
    );
}

#[test]
fn rapid_canonical_pause_burst_does_not_cancel_a_high_latency_local_command() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("high-latency-local-unpause").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );
    coordination.stage_local_pause_intent(false, &session);
    coordination.update_desired_from_session_with_replay(&session, 0.1, false);
    coordination.observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);

    for (received_at, evaluated_at) in [(0.2, 0.2), (-0.1, -0.1), (0.4, 0.4)] {
        session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                received_at,
            )
            .unwrap();
        let actions = coordination.update_desired_from_session(&session, evaluated_at);
        assert!(!has_pause_play_or_seek(&actions));
        assert_eq!(
            coordination.snapshot().pending_local_pause_intent,
            Some(false),
            "packet bursts inside the echo window must retain the in-flight command even across a wall-clock rollback"
        );
    }

    let reconciliation = coordination.update_desired_from_session(&session, 10.3);
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    assert!(
        reconciliation.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(true),
                ..
            }
        )),
        "after both the packet-count and elapsed-time bounds, canonical pause authority must recover"
    );
}

#[test]
fn staged_controller_decision_survives_awaiting_decision_but_not_committed_start() {
    let logical_id = "phase-aware-local-intent";
    let mut awaiting_session = barrier_session();
    awaiting_session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":4.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut awaiting_session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(21, logical_id, 4.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(21),
            )
            .with_status(barrier_status(
                21,
                None,
                PlaybackBarrierPhase::AwaitingDecision,
            )),
    );
    assert_eq!(
        awaiting_session
            .playback_barrier_status()
            .map(|status| status.phase),
        Some(PlaybackBarrierPhase::AwaitingDecision)
    );
    assert!(matches!(
        awaiting_session.current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::ServerBarrier {
            media_generation: 21,
            ..
        })
    ));
    let mut awaiting = RuntimePlaybackCoordination::default();
    awaiting.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    awaiting.update_desired_from_session(&awaiting_session, 0.0);
    awaiting.stage_local_pause_intent(false, &awaiting_session);
    let controller_decision =
        awaiting.update_desired_from_session_with_replay(&awaiting_session, 0.1, false);
    assert!(!has_pause_play_or_seek(&controller_decision));
    assert_eq!(awaiting.snapshot().pending_local_pause_intent, Some(false));
    assert!(
        awaiting
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| !desired.paused && desired.local_echo),
        "a room controller must be able to resolve AwaitingDecision with ordinary play"
    );
    for update_at in [1.0, 6.0, 11.0] {
        awaiting_session
            .apply_message_json_at(
                r#"{"State":{"playstate":{"position":4.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
                update_at,
            )
            .unwrap();
        assert!(matches!(
            awaiting_session.current_room_playstate_authority(),
            Some(RoomPlaystateAuthority::ServerBarrier {
                media_generation: 21,
                ..
            })
        ));
        awaiting.update_desired_from_session(&awaiting_session, update_at);
    }
    assert_eq!(
        awaiting.snapshot().pending_local_pause_intent,
        None,
        "an overlay-capable barrier authority must also retire a repeatedly rejected local command"
    );

    let mut committed_session = barrier_session();
    committed_session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut committed_session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(22, logical_id, 4.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(5),
            )
            .with_status(barrier_status(22, None, PlaybackBarrierPhase::Preparing)),
    );
    apply_barrier_extension(
        &mut committed_session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(22, 5, 4.0, 0.0, 10.0))
            .with_status(barrier_status(22, Some(5), PlaybackBarrierPhase::Committed)),
    );
    assert_eq!(
        committed_session
            .playback_barrier_status()
            .map(|status| status.phase),
        Some(PlaybackBarrierPhase::Committed)
    );
    assert!(matches!(
        committed_session.current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::ServerBarrier {
            media_generation: 22,
            ..
        })
    ));
    let mut committed = RuntimePlaybackCoordination::default();
    committed.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    committed.update_desired_from_session(&committed_session, 0.0);
    committed.stage_local_pause_intent(true, &committed_session);
    let committed_reconciliation =
        committed.update_desired_from_session_with_replay(&committed_session, 0.1, false);
    assert!(!has_pause_play_or_seek(&committed_reconciliation));
    assert_eq!(
        committed.snapshot().pending_local_pause_intent,
        None,
        "Committed remains server-owned until the client observes and acknowledges the synchronized start"
    );
    assert!(
        committed
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| !desired.paused),
        "a local pause must not suppress the canonical committed start before StartedAck"
    );
}

#[test]
fn staged_current_media_unpause_survives_protocol_reconnect_until_matching_echo() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("reconnect-local-intent").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );
    coordination.stage_local_pause_intent(false, &session);
    coordination.update_desired_from_session_with_replay(&session, 0.1, false);

    // A protocol reconnect keeps the user's command for the same media;
    // adapter replacement and media preparation clear it separately.
    coordination.begin_protocol_connection_generation(&session);
    session.reset_sync_state_for_reconnect();
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false)
    );
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
            1.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.1,
        )
        .unwrap();
    let stale_reconnect_state = coordination.update_desired_from_session(&session, 1.1);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false)
    );
    assert!(!stale_reconnect_state.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    let pre_echo_play =
        coordination.observe_transport(transport(1, 1.2, PlayerTransportPhase::Playing, 10.1), 1.2);
    assert!(!pre_echo_play.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            1.3,
        )
        .unwrap();
    coordination.update_desired_from_session(&session, 1.3);
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
}

#[test]
fn controlled_room_reconnect_keeps_pause_intent_dormant_and_auth_failure_discards_it() {
    let mut session = controlled_session_with_authority(true);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("controlled-auth-failure").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.reconcile_external_player_playback(0.0);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );
    runtime.stage_external_player_pause_intent(false, 0.1);
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Playing, 10.0),
        0.1,
    );

    runtime.begin_protocol_connection_generation();
    runtime.session_mut().reset_sync_state_for_reconnect();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
            1.0,
        )
        .unwrap();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"List":{"+room:ABCDEF123456":{"alice":{"controller":false}}}}"#,
            1.05,
        )
        .unwrap();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.1,
        )
        .unwrap();
    let dormant_reconciliation = runtime.reconcile_external_player_playback(1.1);
    assert!(dormant_reconciliation.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        None,
        "dormant semantic intent must not be exposed as an active player override"
    );
    assert!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent_dormant,
        "the semantic command should remain dormant for correlated reauthentication"
    );
    assert_eq!(
        runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .map(|intent| intent.authorization),
        Some(LocalIntentAuthorization::AwaitingControlledRoomReauthentication)
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":false}}}"#,
            1.2,
        )
        .unwrap();
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        None,
        "a correlated authentication failure must permanently discard the stale command"
    );
    runtime.reconcile_external_player_playback(1.2);
    assert!(
        runtime
            .playback_coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused && !desired.local_echo)
    );
}

#[test]
fn controlled_room_reconnect_reauthorizes_same_media_intent_only_after_success() {
    let mut session = controlled_session_with_authority(true);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("controlled-auth-success").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.reconcile_external_player_playback(0.0);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );
    runtime.stage_external_player_pause_intent(false, 0.1);

    runtime.begin_protocol_connection_generation();
    runtime.session_mut().reset_sync_state_for_reconnect();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
            1.0,
        )
        .unwrap();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"List":{"+room:ABCDEF123456":{"alice":{"controller":false}}}}"#,
            1.05,
        )
        .unwrap();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.1,
        )
        .unwrap();
    let dormant_reconciliation = runtime.reconcile_external_player_playback(1.1);
    assert!(
        runtime
            .playback_coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused && !desired.local_echo)
    );
    for action in dormant_reconciliation {
        if let PlaybackCoordinatorAction::Execute { command_id, .. } = action {
            runtime.report_external_coordinator_command_dispatch(command_id, Ok(()), 1.1);
        }
    }
    runtime.observe_external_player_transport(
        paused_transport(1, 1.15, PlayerTransportPhase::ReadyPaused, 10.0),
        1.15,
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":true}}}"#,
            1.2,
        )
        .unwrap();
    let authorized_continuation = runtime.reconcile_external_player_playback(1.2);
    assert!(
        authorized_continuation.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(false)
                    | CoordinatorPlayerCommand::Play(_),
                ..
            }
        )),
        "fresh authority should resume the dormant command: {authorized_continuation:?}"
    );
    let intent = runtime
        .playback_coordination
        .pending_local_pause_intent
        .as_ref()
        .expect("successful reauthentication should safely continue the same intent");
    assert_eq!(intent.authorization, LocalIntentAuthorization::Authorized);
    assert_eq!(
        intent.connection_generation,
        runtime.playback_coordination.connection_generation
    );

    runtime.observe_external_player_transport(
        transport(1, 1.25, PlayerTransportPhase::Playing, 10.1),
        1.25,
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            1.3,
        )
        .unwrap();
    runtime.reconcile_external_player_playback(1.3);
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        None
    );
}

#[test]
fn controlled_room_new_toggle_after_reconnect_waits_for_fresh_authority() {
    let mut runtime =
        controlled_runtime_after_reconnect_without_fresh_authority("controlled-new-toggle");

    let dormant_actions = runtime.stage_external_player_pause_intent(false, 1.1);
    assert!(
        !dormant_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(false)
                    | CoordinatorPlayerCommand::Play(_),
                ..
            }
        )),
        "a cached controller projection must not authorize a new command on the replacement connection"
    );
    let dormant = runtime.playback_coordination_snapshot();
    assert_eq!(dormant.pending_local_pause_intent, None);
    assert!(dormant.pending_local_pause_intent_dormant);
    assert_eq!(dormant.last_local_pause_intent_stage_accepted, Some(true));
    assert!(
        runtime
            .playback_coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused && !desired.local_echo)
    );

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            1.2,
        )
        .unwrap();
    let authorized_actions = runtime.reconcile_external_player_playback(1.2);
    assert_eq!(
        authorized_actions
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(false)
                        | CoordinatorPlayerCommand::Play(_),
                    ..
                }
            ))
            .count(),
        1,
        "fresh correlated Set.user authority should replay the dormant command exactly once: {authorized_actions:?}"
    );
    let authorized = runtime.playback_coordination_snapshot();
    assert_eq!(authorized.pending_local_pause_intent, Some(false));
    assert!(!authorized.pending_local_pause_intent_dormant);

    runtime.observe_external_player_transport(
        transport(1, 1.25, PlayerTransportPhase::Playing, 10.1),
        1.25,
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.1,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            1.3,
        )
        .unwrap();
    runtime.reconcile_external_player_playback(1.3);
    let echoed = runtime.playback_coordination_snapshot();
    assert_eq!(echoed.pending_local_pause_intent, None);
    assert!(!echoed.pending_local_pause_intent_dormant);
}

#[test]
fn controlled_room_fresh_denial_clears_and_rejects_new_reconnect_toggle() {
    let mut runtime =
        controlled_runtime_after_reconnect_without_fresh_authority("controlled-new-toggle-denied");

    runtime.stage_external_player_pause_intent(false, 1.1);
    assert!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent_dormant
    );
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Set":{"controllerAuth":{"room":"+room:ABCDEF123456","user":"alice","success":false}}}"#,
            1.2,
        )
        .unwrap();
    let denied = runtime.playback_coordination_snapshot();
    assert_eq!(denied.pending_local_pause_intent, None);
    assert!(!denied.pending_local_pause_intent_dormant);

    let retry_actions = runtime.stage_external_player_pause_intent(false, 1.3);
    let retry = runtime.playback_coordination_snapshot();
    assert_eq!(retry.last_local_pause_intent_stage_accepted, Some(false));
    assert_eq!(retry.pending_local_pause_intent, None);
    assert!(!retry.pending_local_pause_intent_dormant);
    assert!(
        !retry_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(false)
                    | CoordinatorPlayerCommand::Play(_),
                ..
            }
        )),
        "fresh denial must remain conclusive for the current connection"
    );
}

#[test]
fn uncontrolled_room_new_toggle_after_reconnect_remains_active() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("uncontrolled-new-toggle").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.begin_protocol_connection_generation(&session);
    session.reset_sync_state_for_reconnect();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
            1.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            1.05,
        )
        .unwrap();

    coordination.stage_local_pause_intent(false, &session);
    coordination.update_desired_from_session_with_replay(&session, 1.1, false);
    let snapshot = coordination.snapshot();
    assert_eq!(snapshot.pending_local_pause_intent, Some(false));
    assert!(!snapshot.pending_local_pause_intent_dormant);
}

#[test]
fn playlist_selection_holds_predecessor_play_until_physical_reset_and_new_authority() {
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            0.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":9.5,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":11}}}"#,
            0.1,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"],"user":"bob"}}}"#,
            0.2,
        )
        .unwrap();
    session
        .apply_message_json_at(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#, 0.3)
        .unwrap();
    session
        .apply_message_json_at(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#, 0.4)
        .unwrap();
    assert!(session.has_pending_playlist_index_reset_intent());

    let mut coordination = RuntimePlaybackCoordination::default();
    let plan = coordination.prepare_media(
        LogicalMediaId::new("episode-b.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.4,
    );
    coordination.stage_local_pause_intent(false, &session);
    let initial = coordination.update_desired_from_session(&session, 0.4);
    assert!(initial.iter().all(|action| !matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
    let observed = coordination.observe_transport(
        paused_transport(
            plan.media_generation,
            0.5,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ),
        0.5,
    );
    assert!(observed.iter().all(|action| !matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":12}}}"#,
            0.6,
        )
        .unwrap();
    assert!(session.pending_playlist_index_reset_has_post_selection_playstate());
    let still_held = coordination.update_desired_from_session(&session, 0.6);
    assert!(still_held.iter().all(|action| !matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        Some(false)
    );
    let released = coordination.update_desired_from_session(&session, 0.7);
    assert!(
        released.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPaused(false)
                    | CoordinatorPlayerCommand::Play(_),
                ..
            }
        )),
        "post-selection Play authority may resume only after the physical reset consumes the fence"
    );
}

#[test]
fn playlist_selection_replays_canonical_seek_received_before_physical_reset() {
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            0.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":9.5,"paused":true,"doSeek":false,"setBy":"bob","sorotteTransportRevision":11}}}"#,
            0.1,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"],"user":"bob"}}}"#,
            0.2,
        )
        .unwrap();
    session
        .apply_message_json_at(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#, 0.3)
        .unwrap();
    session
        .apply_message_json_at(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#, 0.4)
        .unwrap();
    assert!(session.has_pending_playlist_index_reset_intent());

    let mut coordination = RuntimePlaybackCoordination::default();
    let plan = coordination.prepare_media(
        LogicalMediaId::new("episode-b.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.4,
    );
    coordination.update_desired_from_session(&session, 0.4);
    coordination.observe_transport(
        paused_transport(
            plan.media_generation,
            0.45,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ),
        0.45,
    );

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":2.0,"paused":true,"doSeek":true,"setBy":"bob","sorotteTransportRevision":12}}}"#,
            0.5,
        )
        .unwrap();
    let held = coordination.update_desired_from_session(&session, 0.5);
    assert!(held.iter().all(|action| !matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position_seconds),
            ..
        } if (*position_seconds - 2.0).abs() <= f64::EPSILON
    )));

    assert_eq!(
        session.take_pending_playlist_index_reset_intent(),
        Some(false)
    );
    let released = coordination.update_desired_from_session(&session, 0.6);
    assert!(
        released.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(position_seconds),
                ..
            } if (*position_seconds - 2.0).abs() <= f64::EPSILON
        )),
        "the newest canonical Seek must replay only after the successor load/reset fence opens"
    );
}

#[test]
fn controlled_room_noncontroller_toggle_is_reconciled_to_canonical_pause() {
    let mut session = controlled_session_with_authority(false);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("noncontroller-toggle").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 10.0),
        0.0,
    );

    coordination.stage_local_pause_intent(false, &session);
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    let correction =
        coordination.observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 10.0), 0.1);
    assert!(correction.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));

    let mut playing_session = controlled_session_with_authority(false);
    playing_session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            2.0,
        )
        .unwrap();
    let mut playing_coordination = RuntimePlaybackCoordination::default();
    playing_coordination.prepare_media(
        LogicalMediaId::new("noncontroller-pause-toggle").unwrap(),
        MediaTransportKind::LocalFile,
        2.0,
    );
    playing_coordination.update_desired_from_session(&playing_session, 2.0);
    playing_coordination
        .observe_transport(transport(1, 2.0, PlayerTransportPhase::Playing, 20.0), 2.0);
    playing_coordination.stage_local_pause_intent(true, &playing_session);
    assert_eq!(
        playing_coordination.snapshot().pending_local_pause_intent,
        None
    );
    let play_correction = playing_coordination.observe_transport(
        paused_transport(1, 2.1, PlayerTransportPhase::ReadyPaused, 20.1),
        2.1,
    );
    assert!(play_correction.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
}

#[test]
fn staged_pause_intent_is_scoped_to_room_and_local_media_generation() {
    let session = controlled_session_with_authority(true);
    let mut coordination = RuntimePlaybackCoordination::default();
    let first = coordination.prepare_media(
        LogicalMediaId::new("scoped-intent-one").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    coordination.stage_local_pause_intent(true, &session);
    let intent = coordination
        .pending_local_pause_intent
        .as_ref()
        .expect("controller command should be staged");
    assert_eq!(intent.room, "+room:ABCDEF123456");
    assert_eq!(intent.local_media_generation, first.media_generation);
    assert_eq!(
        intent.connection_generation,
        coordination.connection_generation
    );

    coordination.handle_authoritative_playback_barrier_room_change();
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
    coordination.stage_local_pause_intent(true, &session);
    coordination.prepare_media(
        LogicalMediaId::new("scoped-intent-two").unwrap(),
        MediaTransportKind::LocalFile,
        1.0,
    );
    assert_eq!(coordination.snapshot().pending_local_pause_intent, None);
}

#[test]
fn authoritative_room_change_force_cancels_dispatched_seek_preparation() {
    let mut coordination = RuntimePlaybackCoordination::default();
    let plan = coordination.prepare_media(
        LogicalMediaId::new("room-scoped-seek").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination
        .coordinator
        .update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: plan.media_generation,
                state_revision: 1,
                paused: false,
                anchor_position_seconds: 40.0,
                anchor_observed_at_seconds: 0.0,
                force_seek: true,
            },
            DesiredRoomPlaybackUpdateKind::ExplicitSeek,
        );
    let actions = coordination.coordinator.observe(
        PlayerTransportObservation::new(plan.media_generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![sorotte_player_api::PlayerSeekableRange::new(
                0.0, 10.0,
            )]),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));

    coordination.handle_authoritative_playback_barrier_room_change();
    let snapshot = coordination.snapshot();
    assert!(snapshot.seek_preparation.is_none());
    assert_eq!(snapshot.last_seek_preparation_terminal_outcome, None);
    assert!(snapshot.last_seek_preparation_terminal.is_none());
}

#[test]
fn startup_barrier_doseek_does_not_enter_client_owned_seek_preparation() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    1,
                    "startup-barrier-media",
                    0.0,
                    PlaybackBarrierPolicy::Controller,
                )
                .with_request_nonce(1),
            )
            .with_status(barrier_status(1, None, PlaybackBarrierPhase::Preparing)),
    );
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("startup-barrier-media").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    assert!(coordination.snapshot().seek_preparation.is_none());

    coordination.observe_transport(
        paused_transport(1, 1.0, PlayerTransportPhase::ReadyPaused, 10.0),
        1.0,
    );
    assert!(session.playback_barrier_prepare().is_some());
    assert!(session.playback_barrier_status().is_some());
    assert!(coordination.current_logical_media_matches("startup-barrier-media"));
    assert!(coordination.latest_observation.is_some());
    let ready = coordination
        .barrier_ready_signature(&session)
        .expect("barrier transport should produce a readiness signature");
    assert!(ready.loaded);
    assert!(
        !ready.buffer_ready,
        "a loaded, paused observation at the wrong target must not satisfy the barrier"
    );
    assert!(coordination.snapshot().seek_preparation.is_none());

    coordination.observe_transport(
        paused_transport(1, 1.1, PlayerTransportPhase::ReadyPaused, 0.0),
        1.1,
    );
    let ready = coordination
        .barrier_ready_signature(&session)
        .expect("the corrected barrier transport should remain reportable");
    assert!(ready.buffer_ready);
    assert!(coordination.snapshot().seek_preparation.is_none());
}

#[test]
fn started_from_precommit_revision_cannot_be_relabelled_as_barrier_ack() {
    let logical_id = "commit-between-start-and-reconciliation";
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    let local_generation = coordination
        .prepare_media(
            LogicalMediaId::new(logical_id).unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        )
        .media_generation;
    coordination.update_desired_from_session(&session, 0.0);
    let precommit_revision = coordination.desired_revision;

    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(31, logical_id, 5.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(7),
            )
            .with_status(barrier_status(31, None, PlaybackBarrierPhase::Preparing)),
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(31, 9, 5.0, 0.0, 10.0))
            .with_status(barrier_status(31, Some(9), PlaybackBarrierPhase::Committed)),
    );

    assert_eq!(
        coordination.barrier_started_target(&session, local_generation, precommit_revision,),
        None,
        "a session commit must not relabel an old coordinator Started action"
    );

    coordination.update_desired_from_session(&session, 0.1);
    let committed_revision = coordination.desired_revision;
    assert_ne!(committed_revision, precommit_revision);
    assert_eq!(
        coordination.barrier_started_target(&session, local_generation, committed_revision,),
        Some((31, 9))
    );
}

#[test]
fn rejected_preparation_seek_projects_runtime_transport_failure() {
    let mut coordination = RuntimePlaybackCoordination::default();
    let generation = coordination
        .prepare_media(
            LogicalMediaId::new("runtime-preparation-failure").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordination
        .coordinator
        .update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: generation,
                state_revision: 1,
                paused: false,
                anchor_position_seconds: 40.0,
                anchor_observed_at_seconds: 0.0,
                force_seek: true,
            },
            DesiredRoomPlaybackUpdateKind::ExplicitSeek,
        );
    let actions = coordination.coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![sorotte_player_api::PlayerSeekableRange::new(
                0.0, 10.0,
            )]),
    );
    let command_id = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPosition(_),
            } => Some(*command_id),
            _ => None,
        })
        .expect("preparation should dispatch a seek");

    coordination.command_dispatch_failed(command_id, 0.2);
    let snapshot = coordination.snapshot();
    assert_eq!(snapshot.diagnostic, PlaybackDiagnostic::Degraded);
    assert_eq!(
        snapshot.last_degraded_reason,
        Some(DegradedPlaybackReason::TransportFailed)
    );
    assert_eq!(
        snapshot.last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
}

#[test]
fn tracked_seek_timeout_retains_preparation_through_extended_deadline_and_late_success() {
    let mut runtime = runtime_with_tracked_fetch_seek();
    runtime
        .player_mut_for_test()
        .command_progress_updates
        .push_back(sorotte_player_api::PlayerCommandProgress::finished(
            PlayerCommandId::new(1),
            Some(PlayerMediaGeneration::new(1)),
            None,
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
        ));
    runtime.drain_player_transport_coordination(15.1).unwrap();
    assert!(
        runtime
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some()
    );
    assert_eq!(
        runtime
            .playback_coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        None
    );

    assert!(runtime.run_keep_waiting_for_seek_preparation(19.0).unwrap());
    assert!(
        runtime
            .playback_coordination
            .coordinator
            .tick(21.0)
            .is_empty(),
        "Keep waiting must move the semantic deadline beyond the adapter's expired tracker"
    );

    let mut ready = paused_transport(1, 25.0, PlayerTransportPhase::ReadyPaused, 40.0);
    ready.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
        35.0, 50.0,
    )]);
    ready.buffered_ahead_seconds = Some(4.0);
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(ready);
    runtime.drain_player_transport_coordination(25.0).unwrap();
    assert_eq!(
        runtime
            .playback_coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn transport_failure_remains_terminal_after_tracked_seek_timeout() {
    let mut runtime = runtime_with_tracked_fetch_seek();
    runtime
        .player_mut_for_test()
        .command_progress_updates
        .push_back(sorotte_player_api::PlayerCommandProgress::finished(
            PlayerCommandId::new(1),
            Some(PlayerMediaGeneration::new(1)),
            None,
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
        ));
    runtime.drain_player_transport_coordination(15.1).unwrap();

    let failed = transport(1, 16.0, PlayerTransportPhase::Failed, 5.0);
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(failed);
    runtime.drain_player_transport_coordination(16.0).unwrap();
    let snapshot = runtime.playback_coordination.snapshot();
    assert_eq!(snapshot.diagnostic, PlaybackDiagnostic::Failed);
    assert_eq!(
        snapshot.last_degraded_reason,
        Some(DegradedPlaybackReason::TransportFailed)
    );
    assert_eq!(
        snapshot.last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
}

#[test]
fn adapter_ready_paused_update_waits_for_delayed_cache_pause_and_release() {
    let mut runtime = runtime_with_tracked_fetch_seek();
    let mut transient = paused_transport(1, 2.0, PlayerTransportPhase::ReadyPaused, 40.0);
    transient.playback_restart_sequence = Some(1);
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(transient);
    runtime.drain_player_transport_coordination(2.0).unwrap();
    assert!(
        runtime
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some()
    );
    assert_eq!(
        runtime
            .playback_coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        None
    );

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            2.1,
            PlayerTransportPhase::Rebuffering,
            40.0,
        ));
    runtime.drain_player_transport_coordination(2.1).unwrap();
    assert!(
        runtime
            .playback_coordination
            .snapshot()
            .seek_preparation
            .is_some()
    );

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            3.0,
            PlayerTransportPhase::ReadyPaused,
            40.0,
        ));
    runtime.drain_player_transport_coordination(3.0).unwrap();
    assert_eq!(
        runtime
            .playback_coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn runtime_merged_replay_cannot_relabel_pre_seek_cache_evidence_as_target_scoped() {
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination
        .coordinator
        .set_config(PlaybackCoordinatorConfig {
            maximum_catchup_rate: 1.25,
            ..PlaybackCoordinatorConfig::default()
        });
    let generation = coordination
        .prepare_media(
            LogicalMediaId::new("runtime-replay-cache-provenance").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    let mut pre_seek = paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0);
    pre_seek.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
        0.0, 10.0,
    )]);
    pre_seek.cache_buffering_percent = Some(100.0);
    pre_seek.buffered_ahead_seconds = Some(10.0);
    pre_seek.input_rate_bytes_per_second = Some(9_000_000);
    coordination.observe_transport(pre_seek, 0.1);
    coordination
        .coordinator
        .update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: generation,
                state_revision: 1,
                paused: false,
                anchor_position_seconds: 40.0,
                anchor_observed_at_seconds: 0.1,
                force_seek: true,
            },
            DesiredRoomPlaybackUpdateKind::ExplicitSeek,
        );
    coordination.observe_transport(
        paused_transport(1, 0.2, PlayerTransportPhase::ReadyPaused, 5.0),
        0.2,
    );
    assert_eq!(
        coordination.snapshot().metrics.last_buffered_ahead_seconds,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .metrics
            .last_input_rate_bytes_per_second,
        None
    );
    coordination.observe_transport(
        paused_transport(1, 2.0, PlayerTransportPhase::Rebuffering, 40.0),
        2.0,
    );
    let mut delayed_after_target = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(2.1)),
    );
    delayed_after_target.seekable_ranges =
        Some(vec![sorotte_player_api::PlayerSeekableRange::new(
            0.0, 10.0,
        )]);
    delayed_after_target.cache_buffering_percent = Some(100.0);
    delayed_after_target.buffered_ahead_seconds = Some(10.0);
    delayed_after_target.input_rate_bytes_per_second = Some(9_000_000);
    coordination.observe_transport(delayed_after_target, 2.1);

    let replay = coordination
        .latest_observation
        .clone()
        .expect("runtime should retain a merged transport snapshot");
    assert_eq!(replay.cache_buffering_percent, Some(100.0));
    assert_eq!(replay.buffered_ahead_seconds, Some(10.0));
    assert_eq!(replay.input_rate_bytes_per_second, Some(9_000_000));
    coordination.coordinator.replay_observation(replay);

    let active = coordination
        .snapshot()
        .seek_preparation
        .expect("cached reconciliation must not complete the preparation");
    assert_eq!(active.cache_buffering_percent, None);
    assert_eq!(active.buffered_ahead_seconds, None);
    assert_eq!(
        coordination.snapshot().metrics.last_buffered_ahead_seconds,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .metrics
            .last_input_rate_bytes_per_second,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        None
    );

    coordination.observe_transport(
        paused_transport(1, 3.0, PlayerTransportPhase::ReadyPaused, 40.0),
        3.0,
    );
    assert_eq!(
        coordination
            .snapshot()
            .last_seek_preparation_terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(
        coordination.snapshot().metrics.last_buffered_ahead_seconds,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .metrics
            .last_input_rate_bytes_per_second,
        None
    );

    let replay_after_ready = coordination
        .latest_observation
        .clone()
        .expect("runtime should retain stale quantitative cache fields after release");
    assert_eq!(replay_after_ready.buffered_ahead_seconds, Some(10.0));
    assert_eq!(
        replay_after_ready.input_rate_bytes_per_second,
        Some(9_000_000)
    );
    coordination
        .coordinator
        .replay_observation(replay_after_ready);
    assert_eq!(
        coordination.snapshot().metrics.last_buffered_ahead_seconds,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .metrics
            .last_input_rate_bytes_per_second,
        None
    );

    let mut fresh_delayed_after_ready = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(3.2)),
    );
    fresh_delayed_after_ready.seekable_ranges =
        Some(vec![sorotte_player_api::PlayerSeekableRange::new(
            0.0, 10.0,
        )]);
    fresh_delayed_after_ready.cache_buffering_percent = Some(100.0);
    fresh_delayed_after_ready.buffered_ahead_seconds = Some(10.0);
    fresh_delayed_after_ready.input_rate_bytes_per_second = Some(9_000_000);
    coordination.observe_transport(fresh_delayed_after_ready, 3.2);
    assert_eq!(
        coordination.snapshot().metrics.last_buffered_ahead_seconds,
        None
    );
    assert_eq!(
        coordination
            .snapshot()
            .metrics
            .last_input_rate_bytes_per_second,
        None
    );

    let catchup =
        coordination.observe_transport(transport(1, 4.0, PlayerTransportPhase::Playing, 40.5), 4.0);
    assert!(catchup.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - 1.03).abs() < f64::EPSILON
    )));
}

#[test]
fn startup_barrier_supersedes_an_active_client_owned_seek_preparation() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":40.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("barrier-supersedes-client-seek").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    let mut initial_transport = paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0);
    initial_transport.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
        0.0, 10.0,
    )]);
    coordination.observe_transport(initial_transport, 0.1);
    assert!(coordination.snapshot().seek_preparation.is_some());

    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    1,
                    "barrier-supersedes-client-seek",
                    5.0,
                    PlaybackBarrierPolicy::Controller,
                )
                .with_request_nonce(1),
            )
            .with_status(barrier_status(1, None, PlaybackBarrierPhase::Preparing)),
    );
    let actions = coordination.update_desired_from_session(&session, 0.2);

    let snapshot = coordination.snapshot();
    assert!(snapshot.seek_preparation.is_none());
    assert!(snapshot.last_seek_preparation_terminal.is_none());
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 5.0).abs() <= f64::EPSILON
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));

    let late_old_seek = coordination.observe_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 40.0),
        0.3,
    );
    assert!(!late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(coordination.snapshot().ordinary_correction_blocked);

    coordination.observe_transport(
        paused_transport(1, 0.4, PlayerTransportPhase::ReadyPaused, 5.0),
        0.4,
    );
    assert!(!coordination.snapshot().ordinary_correction_blocked);
}

#[test]
fn complete_barrier_then_local_pause_retires_committed_play_desire() {
    let logical_id = "terminal-local-pause";
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            0.0,
        )
        .unwrap();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(12, logical_id, 5.0, PlaybackBarrierPolicy::Controller)
                    .with_request_nonce(8),
            )
            .with_commit(CommitStartPayload::new(12, 4, 5.0, 0.0, 10.0))
            .with_status(barrier_status(12, Some(4), PlaybackBarrierPhase::Committed)),
    );
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 5.0), 0.0);
    coordination.update_desired_from_session(&session, 0.0);
    assert_eq!(
        coordination.pending_forced_seek_revision,
        Some(coordination.desired_revision),
        "the committed barrier revision should still be awaiting advancement"
    );

    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(barrier_status(
            12,
            Some(4),
            PlaybackBarrierPhase::Complete,
        )),
    );
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":5.3,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            1.0,
        )
        .unwrap();
    let terminal_pause = coordination.update_desired_from_session(&session, 1.0);

    assert!(!has_pause_play_or_seek(&terminal_pause));
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| {
                desired.paused
                    && desired.barrier_media_generation.is_none()
                    && desired.barrier_state_revision.is_none()
            })
    );
    assert_eq!(coordination.pending_forced_seek_revision, None);
    let confirmed = coordination.observe_transport(
        paused_transport(1, 1.1, PlayerTransportPhase::ReadyPaused, 5.3),
        1.1,
    );
    assert!(!has_pause_play_or_seek(&confirmed));
    assert!(!coordination.snapshot().ordinary_correction_blocked);
}

#[test]
fn buffering_status_supersedes_earlier_self_pause_echo_with_authoritative_alignment() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":8.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("buffering-authority").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 8.0), 0.0);
    coordination.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 8.2), 0.2);

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            1.0,
        )
        .unwrap();
    let self_echo = coordination.update_desired_from_session(&session, 1.0);
    assert!(!has_pause_play_or_seek(&self_echo));
    coordination.stage_local_pause_intent(false, &session);
    let staged_unpause =
        coordination.update_desired_from_session_with_replay(&session, 1.05, false);
    assert!(!has_pause_play_or_seek(&staged_unpause));
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(false)
    );

    let config = RoomBufferingPolicyPayload::new(7, RoomBufferingPolicy::PauseAnyEligible)
        .with_state_revision(3)
        .with_debounce_ms(500)
        .with_resume_hysteresis_ms(1_000)
        .with_max_pause_ms(20_000);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_status(RoomBufferingStatusPayload {
            config,
            phase: RoomBufferingPhase::Paused,
            eligible_clients: 2,
            required_buffering_clients: 1,
            buffering_clients: BTreeSet::from(["bob".to_owned()]),
            pause_deadline: Some(21.0),
        }),
    );
    assert_eq!(
        coordination.active_local_pause_state_mutation_intent(&session),
        None,
        "buffering authority must not let a staged Play mutate canonical pause state"
    );
    let authoritative = coordination.update_desired_from_session(&session, 1.1);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        None,
        "server buffering authority must preempt a staged local unpause"
    );
    assert!(authoritative.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 10.0).abs() < f64::EPSILON
    )));
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| {
                desired.buffering_media_generation == Some(7)
                    && desired.buffering_state_revision == Some(3)
            })
    );

    let aligned =
        coordination.observe_transport(transport(1, 1.2, PlayerTransportPhase::Playing, 10.0), 1.2);
    assert!(aligned.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
}

#[test]
fn explicit_pause_survives_buffering_authority_until_echo_and_player_confirm() {
    let mut session = barrier_session();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":8.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let mut coordination = RuntimePlaybackCoordination::default();
    coordination.prepare_media(
        LogicalMediaId::new("buffering-explicit-pause").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    coordination.update_desired_from_session(&session, 0.0);
    coordination.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 8.0), 0.0);

    let config = RoomBufferingPolicyPayload::new(7, RoomBufferingPolicy::PauseAnyEligible)
        .with_state_revision(3)
        .with_debounce_ms(500)
        .with_resume_hysteresis_ms(1_000)
        .with_max_pause_ms(20_000);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_status(RoomBufferingStatusPayload {
            config,
            phase: RoomBufferingPhase::Paused,
            eligible_clients: 2,
            required_buffering_clients: 1,
            buffering_clients: BTreeSet::from(["bob".to_owned()]),
            pause_deadline: Some(21.0),
        }),
    );

    coordination.stage_local_pause_intent(true, &session);
    assert_eq!(
        coordination
            .active_local_pause_state_mutation_intent(&session)
            .map(|intent| intent.paused),
        Some(true),
        "buffering authority must admit an explicit Pause as a canonical mutation"
    );
    let staged = coordination.update_desired_from_session_with_replay(&session, 0.1, false);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(true),
        "buffering authority must never turn an explicit Pause back into Play"
    );
    assert!(
        coordination
            .desired_fingerprint
            .as_ref()
            .is_some_and(|desired| desired.paused)
    );
    assert!(!staged.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":8.1,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            0.2,
        )
        .unwrap();
    let echo_before_player = coordination.update_desired_from_session(&session, 0.2);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        Some(true),
        "a canonical echo alone must not expose stale playing telemetry"
    );
    assert!(!echo_before_player.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    coordination.observe_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 8.1),
        0.3,
    );
    coordination.update_desired_from_session(&session, 0.3);
    assert_eq!(
        coordination.snapshot().pending_local_pause_intent,
        None,
        "the intent should retire once canonical and physical state both confirm Pause"
    );
}

#[test]
fn system_pause_dispatch_supersedes_user_transport_overlay_and_owns_the_edge() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("system-supersedes-user-transport").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut session = ClientSession::default();
    session.model.room.name = Some("room".to_owned());
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0), 0.0);
    let _ = runtime.classify_latest_player_transition(&session);
    runtime.stage_local_pause_intent(false, &session);
    assert_eq!(runtime.snapshot().pending_local_pause_intent, Some(false));

    let command_id = runtime
        .begin_external_pause_command(PlayerCommandCause::ReadinessGateHold, true, 0.05)
        .expect("the system pause should be registered in the active media scope");
    assert_eq!(runtime.snapshot().pending_local_pause_intent, None);
    assert!(runtime.finish_external_pause_command(command_id, true, 0.06));
    runtime.observe_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.0),
        0.1,
    );
    assert!(matches!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::OwnedCommand {
            command_id: observed_command_id,
            cause: PlayerCommandCause::ReadinessGateHold,
            completion: PlayerCommandCompletion::Completed { .. },
        }) if observed_command_id == command_id
    ));
}

#[test]
fn v2_system_owned_pause_and_play_observations_emit_no_readiness_intent() {
    let cases = [
        (
            PlayerCommandCause::RemoteRoomSynchronization,
            false,
            true,
            UserReadinessIntent::Ready,
        ),
        (
            PlayerCommandCause::AutomaticReadinessStart,
            true,
            false,
            UserReadinessIntent::NotReady,
        ),
        (
            PlayerCommandCause::ReadinessGateHold,
            false,
            true,
            UserReadinessIntent::Ready,
        ),
    ];

    for (cause, initially_paused, commanded_paused, canonical_intent) in cases {
        let mut session = readiness_v2_session_with_intent(1, 41, 0, canonical_intent);
        session.model.playback.local_paused = Some(initially_paused);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            QueuedRuntimeControl::default(),
        );
        runtime.prepare_playback_media(
            LogicalMediaId::new(format!("system-owned-{cause:?}"))
                .expect("test logical ID should be valid"),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        runtime.flush_queued_protocol_messages();

        let baseline = if initially_paused {
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 1.0)
        } else {
            transport(1, 0.0, PlayerTransportPhase::Playing, 1.0)
        };
        runtime.observe_external_player_transport(baseline, 0.0);
        runtime.flush_queued_protocol_messages();

        runtime
            .record_external_system_player_pause_command_result(commanded_paused, cause, true, 0.05)
            .expect("the attached system command should be registered");
        let observed = if commanded_paused {
            paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.0)
        } else {
            transport(1, 0.1, PlayerTransportPhase::Playing, 1.0)
        };
        runtime.observe_external_player_transport(observed, 0.1);

        assert!(matches!(
            runtime
                .playback_coordination
                .last_player_transition_classification,
            Some(PlayerTransitionClassification::OwnedCommand {
                cause: observed_cause,
                ..
            }) if observed_cause == cause
        ));
        assert!(
            runtime.session().pending_readiness_intent().is_none(),
            "{cause:?} telemetry must not stage semantic user readiness"
        );
        assert!(
            runtime.control().outbound_messages().iter().all(|message| {
                let ProtocolMessage::Set(set) = message else {
                    return true;
                };
                set.set
                    .readiness_v2()
                    .expect("readiness extension should decode")
                    .is_none_or(|extension| extension.intent.is_none())
            }),
            "{cause:?} telemetry must not emit a readiness intent"
        );
    }
}

#[test]
fn v2_local_play_correction_is_gate_owned_while_ready_remains_pending() {
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false}}}"#,
            0.0,
        )
        .expect("canonical gate pause should apply");
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(
                    1,
                    "v2-escaped-local-play",
                    1.0,
                    PlaybackBarrierPolicy::AllEligible,
                )
                .with_request_nonce(1),
            )
            .with_status(barrier_status(1, None, PlaybackBarrierPhase::Preparing)),
    );
    session.model.playback.local_paused = Some(false);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("v2-escaped-local-play").expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();
    runtime.observe_external_player_transport(
        transport(1, 0.0, PlayerTransportPhase::Playing, 1.0),
        0.0,
    );
    runtime.flush_queued_protocol_messages();

    assert!(
        runtime
            .run_set_paused(false)
            .expect("V2 Play should stage Ready and restore the gate hold")
    );
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
    );
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|pending| pending.desired()),
        Some(UserReadinessIntent::Ready)
    );

    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.0),
        0.1,
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::OwnedCommand {
            cause: PlayerCommandCause::ReadinessGateHold,
            ..
        })
    ));
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|pending| pending.desired()),
        Some(UserReadinessIntent::Ready),
        "the corrective physical pause must not reverse the user's semantic Play intent"
    );
}

#[test]
fn v2_pending_native_play_can_be_confirmed_once_and_emits_ready() {
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(PrepareMediaPayload::new(
                1,
                "explicit-native-play",
                1.0,
                PlaybackBarrierPolicy::Controller,
            ))
            .with_status(barrier_status(1, None, PlaybackBarrierPhase::Preparing)),
    );
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("explicit-native-play").expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 1.0),
        0.0,
    );
    runtime.flush_queued_protocol_messages();
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Playing, 1.0),
        0.1,
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::AwaitingStability {
            action: NativePlayerAction::Play,
            ..
        })
    ));
    assert!(runtime.session().pending_readiness_intent().is_none());
    runtime.flush_queued_protocol_messages();

    assert!(
        runtime
            .confirm_pending_native_player_play(PlayerInteractionSurface::NativePlayerControl,)
            .expect("the explicit native Play should dispatch")
    );
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|pending| pending.desired()),
        Some(UserReadinessIntent::Ready)
    );
    let intents = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .and_then(|extension| extension.intent)
        })
        .collect::<Vec<_>>();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].desired, UserReadinessIntent::Ready);
    assert_eq!(
        intents[0].source,
        UserReadinessMutationSource::IndirectPlayer {
            action: sorotte_protocol::PlayerReadinessAction::Play,
            surface: PlayerInteractionSurface::NativePlayerControl,
        }
    );

    assert!(
        !runtime
            .confirm_pending_native_player_play(PlayerInteractionSurface::NativePlayerControl,)
            .expect("a consumed edge should be a harmless no-op")
    );
    let intent_count = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter(|message| {
            let ProtocolMessage::Set(set) = message else {
                return false;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .is_some_and(|extension| extension.intent.is_some())
        })
        .count();
    assert_eq!(intent_count, 1);
}

#[test]
fn managed_player_single_native_play_edge_emits_one_ready_before_gate_hold_pause() {
    let logical_id = "managed-single-native-play";
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false}}}"#,
            0.0,
        )
        .expect("canonical gate pause should apply");
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(1, logical_id, 1.0, PlaybackBarrierPolicy::AllEligible)
                    .with_request_nonce(1),
            )
            .with_status(barrier_status(1, None, PlaybackBarrierPhase::Preparing)),
    );
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new(logical_id).expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            0.0,
            PlayerTransportPhase::ReadyPaused,
            1.0,
        ));
    runtime
        .drain_player_transport_coordination(0.0)
        .expect("paused baseline should reconcile");
    runtime.flush_queued_protocol_messages();
    runtime.player_mut_for_test().commands.clear();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(transport(1, 0.1, PlayerTransportPhase::Playing, 1.0));
    runtime
        .drain_player_transport_coordination(0.1)
        .expect("one native Playing edge should be preserved before correction");

    let intents = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .and_then(|extension| extension.intent)
        })
        .collect::<Vec<_>>();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].desired, UserReadinessIntent::Ready);
    assert_eq!(
        intents[0].source,
        UserReadinessMutationSource::IndirectPlayer {
            action: sorotte_protocol::PlayerReadinessAction::Play,
            surface: PlayerInteractionSurface::NativePlayerControl,
        }
    );
    assert!(
        runtime.control().outbound_messages().iter().all(|message| {
            let ProtocolMessage::State(state) = message else {
                return true;
            };
            state
                .state
                .playstate
                .as_ref()
                .and_then(|playstate| playstate.paused)
                != Some(false)
        }),
        "the held native edge must not publish physical Play before CommitStart"
    );
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPaused(true)))
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::NativePlayerGesture {
            action: NativePlayerAction::Play
        })
    ));

    runtime
        .drain_player_transport_coordination(0.2)
        .expect("an empty follow-up pump should be harmless");
    let intent_count = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter(|message| {
            let ProtocolMessage::Set(set) = message else {
                return false;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .is_some_and(|extension| extension.intent.is_some())
        })
        .count();
    assert_eq!(intent_count, 1, "the same edge must not emit twice");
}

fn assert_managed_native_play_survives_manual_v2_phase(
    logical_id: &str,
    barrier_phase: Option<PlaybackBarrierPhase>,
) {
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .expect("canonical user pause should apply");
    if let Some(barrier_phase) = barrier_phase {
        apply_barrier_extension(
            &mut session,
            PlaybackBarrierSetExtension::new()
                .with_prepare(
                    PrepareMediaPayload::new(
                        1,
                        logical_id,
                        1.0,
                        PlaybackBarrierPolicy::AllEligible,
                    )
                    .with_request_nonce(1),
                )
                .with_status(barrier_status(1, None, barrier_phase)),
        );
        assert_eq!(
            session.playback_barrier_status().map(|status| status.phase),
            Some(barrier_phase),
        );
    }
    assert_eq!(session.local_can_control(), Some(true));
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new(logical_id).expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();
    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            0.0,
            PlayerTransportPhase::ReadyPaused,
            1.0,
        ));
    runtime
        .drain_player_transport_coordination(0.0)
        .expect("paused baseline should reconcile");
    runtime.flush_queued_protocol_messages();
    runtime.player_mut_for_test().commands.clear();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(transport(1, 0.1, PlayerTransportPhase::Playing, 1.0));
    runtime
        .drain_player_transport_coordination(0.1)
        .expect("the native Play should become controller-owned");

    assert!(
        runtime
            .player()
            .commands
            .iter()
            .all(|command| !matches!(command, PlayerCommand::SetPaused(true))),
        "manual V2 phases must not erase the sole native Playing edge"
    );
    let intent_count = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter(|message| {
            let ProtocolMessage::Set(set) = message else {
                return false;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .is_some_and(|extension| extension.intent.is_some())
        })
        .count();
    assert_eq!(
        intent_count, 1,
        "one native Playing edge must be promoted before canonical correction"
    );

    let intents = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .and_then(|extension| extension.intent)
        })
        .collect::<Vec<_>>();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].desired, UserReadinessIntent::Ready);
    assert_eq!(
        intents[0].source,
        UserReadinessMutationSource::IndirectPlayer {
            action: sorotte_protocol::PlayerReadinessAction::Play,
            surface: PlayerInteractionSurface::NativePlayerControl,
        }
    );
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .all(|command| !matches!(command, PlayerCommand::SetPaused(true)))
    );
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false),
        "the native controller must own transport until its canonical echo"
    );

    runtime
        .drain_player_transport_coordination(0.3)
        .expect("a follow-up pump should preserve the local transport overlay");
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .all(|command| !matches!(command, PlayerCommand::SetPaused(true)))
    );
}

#[test]
fn managed_native_play_survives_ordinary_post_start_pause() {
    assert_managed_native_play_survives_manual_v2_phase("managed-post-start-play", None);
}

#[test]
fn managed_native_play_resolves_awaiting_decision_without_repause() {
    assert_managed_native_play_survives_manual_v2_phase(
        "managed-awaiting-decision-play",
        Some(PlaybackBarrierPhase::AwaitingDecision),
    );
}

#[test]
fn managed_native_play_recovers_terminal_timeout_without_repause() {
    assert_managed_native_play_survives_manual_v2_phase(
        "managed-terminal-timeout-play",
        Some(PlaybackBarrierPhase::Degraded),
    );
}

#[test]
fn native_play_after_seek_position_convergence_beats_stale_pause_correction() {
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":11.0,"paused":true,"doSeek":true,"setBy":"bob","sorotteTransportRevision":24}}}"#,
            0.0,
        )
        .expect("canonical paused seek should apply");
    assert_eq!(session.local_can_control(), Some(true));
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("final-item-seek-play-race").expect("logical ID should be valid"),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.flush_queued_protocol_messages();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            0.0,
            PlayerTransportPhase::ReadyPaused,
            0.0,
        ));
    runtime
        .drain_player_transport_coordination(0.0)
        .expect("paused origin should dispatch canonical seek");
    assert!(runtime.player().commands.iter().any(
        |command| matches!(command, PlayerCommand::SetPosition(position) if (*position - 11.0).abs() <= f64::EPSILON)
    ));
    runtime.flush_queued_protocol_messages();
    runtime.player_mut_for_test().commands.clear();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(paused_transport(
            1,
            0.1,
            PlayerTransportPhase::ReadyPaused,
            11.0,
        ));
    runtime
        .drain_player_transport_coordination(0.1)
        .expect("target position should converge before command completion arrives");
    assert!(
        !runtime
            .playback_coordination
            .player_command_bindings
            .is_empty(),
        "the regression requires command completion to remain in flight"
    );
    assert!(
        runtime
            .playback_coordination
            .player_command_bindings
            .values()
            .all(|binding| binding.desired_paused.is_none())
    );
    runtime.flush_queued_protocol_messages();
    runtime.player_mut_for_test().commands.clear();

    runtime
        .player_mut_for_test()
        .transport_updates
        .push_back(transport(1, 0.2, PlayerTransportPhase::Playing, 11.0));
    runtime
        .drain_player_transport_coordination(0.2)
        .expect("native Play should supersede stale canonical pause correction");

    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::NativePlayerGesture {
            action: NativePlayerAction::Play
        })
    ));
    assert!(
        runtime
            .player()
            .commands
            .iter()
            .all(|command| !matches!(command, PlayerCommand::SetPaused(true))),
        "an in-flight seek completion must not cause immediate re-pause"
    );
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false),
        "native Play must remain authoritative until the server commits it"
    );
    assert!(runtime.control().outbound_messages().iter().any(|message| {
        let ProtocolMessage::Set(set) = message else {
            return false;
        };
        set.set
            .readiness_v2()
            .expect("readiness extension should decode")
            .and_then(|extension| extension.intent)
            .is_some_and(|intent| intent.desired == UserReadinessIntent::Ready)
    }));
}

#[test]
fn pre_authority_native_play_cannot_suppress_later_remote_pause() {
    let logical_id = "native-play-before-room-authority";
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    assert_eq!(session.local_can_control(), Some(true));
    assert!(session.current_room_playstate().is_none());
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new(logical_id).expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();

    assert!(
        runtime
            .observe_external_player_transport(
                paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 1.0),
                0.0,
            )
            .is_empty(),
        "the paused player baseline needs no correction before room authority arrives"
    );
    runtime.flush_queued_protocol_messages();
    assert!(
        runtime
            .observe_external_player_transport(
                transport(1, 0.1, PlayerTransportPhase::Playing, 1.0),
                0.1,
            )
            .is_empty(),
        "one pre-authority Playing edge must remain unconfirmed"
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::AwaitingStability {
            action: NativePlayerAction::Play,
            first_observed_at_seconds: 0.1,
        })
    ));
    assert!(runtime.session().pending_readiness_intent().is_none());
    runtime.flush_queued_protocol_messages();

    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            0.2,
        )
        .expect("Bob's authoritative room pause should apply");
    let correction = runtime.reconcile_external_player_playback(0.2);
    let pause_command_id = correction
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPaused(true),
            } => Some(*command_id),
            _ => None,
        })
        .expect("newer remote pause authority must retain its player correction");
    runtime.observe_external_player_transport(
        transport(1, 0.22, PlayerTransportPhase::Playing, 1.0),
        0.22,
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::Duplicate)
    ));
    assert!(runtime.control().outbound_messages().iter().all(|message| {
        let ProtocolMessage::Set(set) = message else {
            return true;
        };
        set.set
            .readiness_v2()
            .expect("readiness extension should decode")
            .is_none_or(|extension| extension.intent.is_none())
    }));
    let room_playstate = runtime
        .session()
        .current_room_playstate()
        .expect("Bob's playstate should remain canonical");
    assert_eq!(room_playstate.paused, Some(true));
    assert_eq!(room_playstate.set_by.as_deref(), Some("bob"));

    let player_command_id = runtime
        .begin_external_coordinator_command_dispatch(pause_command_id, 0.23)
        .expect("the retained pause correction should register causal ownership");
    runtime.finish_external_coordinator_command_dispatch(
        pause_command_id,
        Some(player_command_id),
        Ok(()),
        0.23,
    );
    runtime.observe_external_player_transport(
        paused_transport(1, 0.25, PlayerTransportPhase::ReadyPaused, 1.0),
        0.25,
    );
    runtime.flush_queued_protocol_messages();

    let fresh_play_actions = runtime.observe_external_player_transport(
        transport(1, 0.3, PlayerTransportPhase::Playing, 1.0),
        0.3,
    );
    assert!(fresh_play_actions.iter().all(|action| !matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    let intents = runtime
        .control()
        .outbound_messages()
        .iter()
        .filter_map(|message| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            set.set
                .readiness_v2()
                .expect("readiness extension should decode")
                .and_then(|extension| extension.intent)
        })
        .collect::<Vec<_>>();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].desired, UserReadinessIntent::Ready);
    assert_eq!(
        intents[0].source,
        UserReadinessMutationSource::IndirectPlayer {
            action: sorotte_protocol::PlayerReadinessAction::Play,
            surface: PlayerInteractionSurface::NativePlayerControl,
        }
    );
}

#[test]
fn new_scope_initial_observation_cannot_be_confirmed_as_native_play() {
    let mut session = readiness_v2_session_with_intent(1, 41, 0, UserReadinessIntent::NotReady);
    session.model.playback.local_paused = Some(true);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("old-native-play-scope").expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 1.0),
        0.0,
    );
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Playing, 1.0),
        0.1,
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::AwaitingStability { .. })
    ));

    runtime.prepare_playback_media(
        LogicalMediaId::new("new-native-play-scope").expect("logical ID should be valid"),
        MediaTransportKind::NetworkVod,
        0.2,
    );
    runtime.flush_queued_protocol_messages();
    runtime.observe_external_player_transport(
        transport(2, 0.2, PlayerTransportPhase::Playing, 1.0),
        0.2,
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::UnknownOrigin {
            reason: crate::PlayerTransitionUnknownReason::InitialObservation,
            ..
        })
    ));
    runtime.flush_queued_protocol_messages();

    assert!(
        !runtime
            .confirm_pending_native_player_play(PlayerInteractionSurface::NativePlayerControl,)
            .expect("an initial observation should be a harmless no-op")
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(runtime.control().outbound_messages().iter().all(|message| {
        let ProtocolMessage::Set(set) = message else {
            return true;
        };
        set.set
            .readiness_v2()
            .expect("readiness extension should decode")
            .is_none_or(|extension| extension.intent.is_none())
    }));
}

#[test]
fn periodic_desired_reconciliation_does_not_advance_recovery_stability() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut session = ClientSession::default();
    session.model.connection.username = Some("alice".to_owned());
    session.model.room.name = Some("room".to_owned());
    session.model.room.playstates.insert(
        "room".to_owned(),
        RoomPlaystateView {
            paused: Some(false),
            position: Some(40.0),
            set_by: Some("bob".to_owned()),
            ..RoomPlaystateView::default()
        },
    );
    session
        .model
        .room
        .playstate_updated_at_seconds
        .insert("room".to_owned(), 10.0);
    runtime.update_desired_from_session(&session, 10.0);
    // Treat the initial media-position handoff as already satisfied; this
    // test isolates the recovery episode rather than startup seeking.
    runtime.pending_forced_seek_revision = None;
    runtime.update_desired_from_session(&session, 10.0);
    runtime.observe_transport(
        transport(1, 10.0, PlayerTransportPhase::Rebuffering, 10.0),
        10.0,
    );
    runtime.observe_transport(
        transport(1, 11.0, PlayerTransportPhase::Playing, 10.2),
        11.0,
    );
    runtime.observe_transport(
        transport(1, 12.0, PlayerTransportPhase::Playing, 10.5),
        12.0,
    );
    let episode = runtime
        .snapshot()
        .recovery_episode
        .expect("buffer recovery should remain active");
    assert_eq!(episode.hard_seek_attempts, 1);

    for now_seconds in [12.5, 13.0, 14.0, 15.0] {
        runtime.update_desired_from_session(&session, now_seconds);
    }

    let after = runtime
        .snapshot()
        .recovery_episode
        .expect("reconciliation without a fresh observation cannot close recovery");
    assert_eq!(after.id, episode.id);
    assert_eq!(after.hard_seek_attempts, 1);
    assert_eq!(runtime.snapshot().metrics.hard_seek_count, 1);
}

#[test]
fn player_transition_runtime_retains_timed_out_command_ownership_for_late_telemetry() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("command-owned-transition").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let session = ClientSession::default();
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0), 0.0);
    assert!(matches!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::UnknownOrigin {
            reason: crate::PlayerTransitionUnknownReason::InitialObservation,
            ..
        })
    ));

    let command_id = PlayerCommandId::new(77);
    runtime.bind_standalone_player_pause_command(
        command_id,
        PlayerCommandCause::RemoteRoomSynchronization,
        true,
        0.05,
    );
    runtime.apply_player_command_progress(
        sorotte_player_api::PlayerCommandProgress::finished(
            command_id,
            Some(PlayerMediaGeneration::new(1)),
            Some(PlayerObservationTimestamp::from_adapter_start(
                Duration::from_secs_f64(0.1),
            )),
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TimedOut),
        ),
        0.1,
    );
    runtime.observe_transport(
        paused_transport(1, 0.2, PlayerTransportPhase::ReadyPaused, 1.0),
        0.2,
    );

    assert!(matches!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::OwnedCommand {
            command_id: owned_id,
            cause: PlayerCommandCause::RemoteRoomSynchronization,
            completion: PlayerCommandCompletion::TimedOut { .. },
        }) if owned_id == command_id
    ));
}

#[test]
fn completion_not_observed_outcome_terminally_removes_client_core_command_binding() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let player_command_id = PlayerCommandId::new(81);
    runtime.bind_player_command(
        player_command_id,
        CoordinatorCommandId::new(17),
        PlayerCommandCause::RemoteRoomSynchronization,
        None,
        0.0,
    );
    assert!(
        runtime
            .player_command_bindings
            .contains_key(&player_command_id)
    );

    runtime.apply_player_command_outcome(
        sorotte_player_api::PlayerCommandOutcome {
            attachment_epoch: sorotte_player_api::PlayerAttachmentEpoch::new(1),
            command_id: player_command_id,
            media_generation: Some(PlayerMediaGeneration::new(3)),
            result: sorotte_player_api::PlayerCommandSemanticResult::CompletionNotObserved,
        },
        1.0,
    );

    assert!(
        !runtime
            .player_command_bindings
            .contains_key(&player_command_id)
    );
}

#[test]
fn attached_user_command_result_owns_following_player_edge() {
    for succeeded in [true, false] {
        let mut runtime = RuntimePlaybackCoordination::default();
        runtime.prepare_media(
            LogicalMediaId::new("attached-user-transition").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        let session = ClientSession::default();
        runtime.observe_transport(
            paused_transport(1, 0.0, PlayerTransportPhase::ReadyPaused, 1.0),
            0.0,
        );
        let _ = runtime.classify_latest_player_transition(&session);

        runtime.register_external_pause_command_result(
            PlayerCommandCause::LocalUserPlaybackControl,
            false,
            succeeded,
            0.05,
        );
        runtime.observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 1.0), 0.1);
        let classification = runtime
            .classify_latest_player_transition(&session)
            .expect("the command-owned edge should be classified");
        assert!(
            matches!(
                classification,
                PlayerTransitionClassification::OwnedCommand {
                    cause: PlayerCommandCause::LocalUserPlaybackControl,
                    completion: PlayerCommandCompletion::Completed { .. },
                    ..
                } if succeeded
            ) || matches!(
                classification,
                PlayerTransitionClassification::OwnedCommand {
                    cause: PlayerCommandCause::LocalUserPlaybackControl,
                    completion: PlayerCommandCompletion::Failed { .. },
                    ..
                } if !succeeded
            )
        );

        runtime.observe_transport(
            transport(1, 0.25, PlayerTransportPhase::Playing, 1.15),
            0.25,
        );
        assert_eq!(
            runtime.classify_latest_player_transition(&session),
            Some(PlayerTransitionClassification::Duplicate),
            "the same Sorotte-issued edge must not become a native gesture"
        );
    }
}

#[test]
fn immediately_failed_sorotte_pause_owns_a_late_player_edge() {
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        CoordinatedTestPlayer {
            reject_pause_commands: true,
            ..CoordinatedTestPlayer::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("immediate-failure-causal-owner").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime
        .playback_coordination
        .observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0), 0.0);
    let _ = runtime
        .playback_coordination
        .classify_latest_player_transition(&runtime.session);

    runtime
        .execute_causal_pause_command(true, PlayerCommandCause::RemoteRoomSynchronization, 0.05)
        .expect_err("the test adapter should reject the tracked pause");
    runtime.playback_coordination.observe_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.0),
        0.1,
    );

    assert!(matches!(
        runtime
            .playback_coordination
            .classify_latest_player_transition(&runtime.session),
        Some(PlayerTransitionClassification::OwnedCommand {
            cause: PlayerCommandCause::RemoteRoomSynchronization,
            completion: PlayerCommandCompletion::Failed { .. },
            ..
        })
    ));
}

#[test]
fn asynchronous_pause_failure_reports_a_technical_block() {
    let session = readiness_v2_session(1, 41, 0);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("async-failure-technical-block").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.flush_queued_protocol_messages();

    runtime
        .execute_causal_pause_command(true, PlayerCommandCause::LocalUserPlaybackControl, 0.05)
        .expect("the tracked pause should initially be accepted");
    runtime
        .player_mut_for_test()
        .command_progress_updates
        .push_back(sorotte_player_api::PlayerCommandProgress::finished(
            PlayerCommandId::new(1),
            Some(PlayerMediaGeneration::new(1)),
            Some(PlayerObservationTimestamp::from_adapter_start(
                Duration::from_secs_f64(0.1),
            )),
            None,
            PlayerCommandResult::Failed(PlayerCommandFailureKind::TransportDisconnected),
        ));

    runtime
        .drain_player_transport_coordination(0.1)
        .expect("technical failure reporting should succeed");
    let technical = runtime
        .control()
        .outbound_messages()
        .iter()
        .find_map(|message| {
            let ProtocolMessage::State(state) = message else {
                return None;
            };
            state
                .state
                .readiness_v2()
                .expect("readiness extension should decode")
                .and_then(|extension| extension.technical)
        });
    let technical = technical.expect("the asynchronous failure should publish a block");
    assert_eq!(
        technical.phase,
        TechnicalPlayabilityPhase::TemporarilyBlocked
    );
    assert_eq!(technical.reason, Some(TechnicalBlockCause::PlayerFailure));
}

#[test]
fn player_transition_runtime_requires_two_stable_unowned_samples_for_native_gesture() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("native-transition").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let session = ClientSession::default();
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 2.0), 0.0);
    let _ = runtime.classify_latest_player_transition(&session);

    runtime.observe_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 2.0),
        0.1,
    );
    assert!(matches!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::AwaitingStability {
            action: NativePlayerAction::Pause,
            ..
        })
    ));

    runtime.observe_transport(
        paused_transport(1, 0.25, PlayerTransportPhase::ReadyPaused, 2.0),
        0.25,
    );
    assert_eq!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::NativePlayerGesture {
            action: NativePlayerAction::Pause,
        })
    );
}

#[test]
fn committed_barrier_does_not_swallow_a_native_pause_after_automatic_start() {
    let logical_id = "native-pause-after-commit";
    let mut session = barrier_session();
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(9, logical_id, 0.0, PlaybackBarrierPolicy::AllEligible)
                    .with_request_nonce(1),
            )
            .with_status(barrier_status(9, None, PlaybackBarrierPhase::Preparing)),
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(9, 3, 0.0, 0.0, 10.0))
            .with_status(barrier_status(9, Some(3), PlaybackBarrierPhase::Committed)),
    );

    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 0.1), 0.0);
    let _ = runtime.classify_latest_player_transition(&session);
    runtime.observe_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 0.1),
        0.1,
    );
    assert!(matches!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::AwaitingStability {
            action: NativePlayerAction::Pause,
            ..
        })
    ));
    runtime.observe_transport(
        paused_transport(1, 0.25, PlayerTransportPhase::ReadyPaused, 0.1),
        0.25,
    );
    assert_eq!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::NativePlayerGesture {
            action: NativePlayerAction::Pause,
        })
    );
}

#[test]
fn technical_readiness_reports_semantic_state_changes_and_deduplicates_exact_repeats() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let session = readiness_v2_session(1, 41, 0);
    runtime.prepare_media(
        LogicalMediaId::new("technical-readiness").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Loading, 0.0), 0.0);
    let preparing = runtime
        .next_technical_readiness_report(&session)
        .expect("loading should report preparing");
    assert_eq!(preparing.phase, TechnicalPlayabilityPhase::Preparing);
    assert_eq!(preparing.reason, Some(TechnicalBlockCause::Loading));
    runtime.mark_technical_readiness_report_delivered(&preparing);

    runtime.observe_transport(
        transport(1, 0.1, PlayerTransportPhase::Prebuffering, 0.0),
        0.1,
    );
    let prebuffering = runtime
        .next_technical_readiness_report(&session)
        .expect("a same-phase cause change must be reported");
    assert_eq!(prebuffering.phase, TechnicalPlayabilityPhase::Preparing);
    assert_eq!(prebuffering.reason, Some(TechnicalBlockCause::Prebuffering));
    runtime.mark_technical_readiness_report_delivered(&prebuffering);
    assert_eq!(runtime.next_technical_readiness_report(&session), None);

    runtime.observe_transport(transport(1, 0.2, PlayerTransportPhase::Playing, 0.1), 0.2);
    let playable = runtime
        .next_technical_readiness_report(&session)
        .expect("stable playback should report playable");
    assert_eq!(playable.phase, TechnicalPlayabilityPhase::Playable);
    assert_eq!(playable.reason, None);
    runtime.mark_technical_readiness_report_delivered(&playable);

    runtime.observe_transport(transport(1, 0.3, PlayerTransportPhase::Failed, 0.1), 0.3);
    let terminal = runtime
        .next_technical_readiness_report(&session)
        .expect("player failure should report a terminal block");
    assert_eq!(terminal.phase, TechnicalPlayabilityPhase::TerminallyBlocked);
    assert_eq!(terminal.reason, Some(TechnicalBlockCause::PlayerFailure));
    runtime.mark_technical_readiness_report_delivered(&terminal);
}

#[test]
fn post_commit_rebuffer_uses_server_revision_despite_divergent_local_revision() {
    let logical_id = "technical-authoritative-revision";
    let mut session = readiness_v2_session(22, 41, 0);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(
                PrepareMediaPayload::new(22, logical_id, 0.0, PlaybackBarrierPolicy::AllEligible)
                    .with_request_nonce(5),
            )
            .with_status(barrier_status(22, None, PlaybackBarrierPhase::Preparing)),
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(22, 17, 0.0, 0.0, 10.0))
            .with_status(barrier_status(
                22,
                Some(17),
                PlaybackBarrierPhase::Committed,
            )),
    );
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(barrier_status(
            22,
            Some(17),
            PlaybackBarrierPhase::Complete,
        )),
    );

    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new(logical_id).unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    runtime.last_applied_revision = Some(4);
    let mut rebuffering = transport(1, 0.5, PlayerTransportPhase::Rebuffering, 3.0);
    rebuffering.paused_for_cache = Some(false);
    runtime.observe_transport(rebuffering, 0.5);

    let report = runtime
        .next_technical_readiness_report(&session)
        .expect("post-commit rebuffering should produce a fenced report");
    assert_eq!(report.media_generation, 22);
    assert_eq!(report.phase, TechnicalPlayabilityPhase::TemporarilyBlocked);
    assert!(matches!(
        report.reason,
        Some(TechnicalBlockCause::Rebuffering | TechnicalBlockCause::Recovery)
    ));
    assert_eq!(report.authoritative_playback_revision, Some(17));
    assert_ne!(
        report.authoritative_playback_revision,
        runtime.last_applied_revision
    );

    runtime.mark_technical_readiness_report_delivered(&report);
    apply_barrier_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(CommitStartPayload::new(22, 18, 0.0, 0.0, 10.0))
            .with_status(barrier_status(22, Some(18), PlaybackBarrierPhase::Complete)),
    );
    let revised = runtime
        .next_technical_readiness_report(&session)
        .expect("a new authoritative revision must resend unchanged technical state");
    assert_eq!(revised.authoritative_playback_revision, Some(18));
    assert!(revised.report_sequence > report.report_sequence);
}

#[test]
fn unchanged_technical_state_resends_for_connection_membership_and_revision_changes() {
    let mut runtime = RuntimePlaybackCoordination::default();
    runtime.prepare_media(
        LogicalMediaId::new("technical-resend-scope").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0), 0.0);
    let session = readiness_v2_session(1, 41, 0);
    let first = runtime
        .next_technical_readiness_report(&session)
        .expect("first membership observation should report");
    runtime.mark_technical_readiness_report_delivered(&first);

    runtime.begin_protocol_connection_generation(&session);
    let reconnect = runtime
        .next_technical_readiness_report(&session)
        .expect("a new protocol connection must resend unchanged state");
    assert_eq!(reconnect.membership_epoch, 41);
    assert!(reconnect.report_sequence > first.report_sequence);
    runtime.mark_technical_readiness_report_delivered(&reconnect);

    let replacement = readiness_v2_session(1, 73, 9);
    let new_membership = runtime
        .next_technical_readiness_report(&replacement)
        .expect("a fresh membership baseline must resend unchanged state");
    assert_eq!(new_membership.membership_epoch, 73);
    assert_eq!(new_membership.report_sequence, 10);
}

#[test]
fn failed_technical_readiness_delivery_retries_an_identical_report() {
    #[derive(Default)]
    struct RejectFirstTechnicalControl {
        attempts: usize,
    }

    impl ClientEffectSink for RejectFirstTechnicalControl {
        fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
            if matches!(effect, ClientEffect::ReportTechnicalReadiness(_)) {
                self.attempts += 1;
                if self.attempts == 1 {
                    return Err(ClientEffectError::OperationFailed(
                        "forced technical readiness delivery failure".to_owned(),
                    ));
                }
            }
            Ok(())
        }
    }

    let session = readiness_v2_session(1, 41, 0);
    let mut runtime = ClientRuntime::new(
        session,
        DisconnectedPlayer,
        RejectFirstTechnicalControl::default(),
    );
    runtime.playback_coordination.prepare_media(
        LogicalMediaId::new("technical-delivery-retry").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime
        .playback_coordination
        .observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0), 0.0);

    runtime
        .handle_latest_player_readiness_observation()
        .expect_err("the first technical report should be rejected");
    assert_eq!(runtime.control.attempts, 1);

    runtime
        .playback_coordination
        .observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 1.1), 0.1);
    runtime
        .handle_latest_player_readiness_observation()
        .expect("the identical technical report should retry successfully");
    assert_eq!(runtime.control.attempts, 2);

    runtime
        .handle_latest_player_readiness_observation()
        .expect("a delivered identical report should deduplicate");
    assert_eq!(runtime.control.attempts, 2);
}

#[test]
fn technical_readiness_treats_eof_as_transition_not_terminal_failure() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let session = readiness_v2_session(1, 41, 0);
    runtime.prepare_media(
        LogicalMediaId::new("technical-readiness-eof").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Ended, 90.0), 0.0);

    let report = runtime
        .next_technical_readiness_report(&session)
        .expect("EOF should publish the technical transition");
    assert_eq!(report.phase, TechnicalPlayabilityPhase::Preparing);
    assert_eq!(report.reason, Some(TechnicalBlockCause::EndOfFile));
    assert_eq!(report.recovery, Some(RecoveryStage::NotStarted));
}

#[test]
fn legacy_external_eof_is_classified_as_technical_before_native_gesture_detection() {
    let mut runtime = RuntimePlaybackCoordination::default();
    let readiness_session = readiness_v2_session(1, 41, 0);
    runtime.prepare_media(
        LogicalMediaId::new("legacy-external-eof").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    let session = ClientSession::default();
    runtime.observe_transport(transport(1, 0.0, PlayerTransportPhase::Playing, 89.0), 0.0);
    let _ = runtime.classify_latest_player_transition(&session);

    runtime.observe_external_end_of_file(0.5);

    assert_eq!(
        runtime.classify_latest_player_transition(&session),
        Some(PlayerTransitionClassification::Technical {
            action: NativePlayerAction::Pause,
            reason: crate::PlayerTransitionTechnicalReason::Ended,
        })
    );
    let report = runtime
        .next_technical_readiness_report(&readiness_session)
        .expect("legacy EOF should publish a technical readiness transition");
    assert_eq!(report.phase, TechnicalPlayabilityPhase::Preparing);
    assert_eq!(report.reason, Some(TechnicalBlockCause::EndOfFile));
}

#[test]
fn failed_transport_during_active_recovery_is_temporarily_blocked() {
    let mut runtime = coordination_with_owned_catchup_rate();
    let session = readiness_v2_session(1, 41, 0);
    assert!(runtime.snapshot().recovery_episode.is_some());
    runtime.observe_transport(transport(1, 13.1, PlayerTransportPhase::Failed, 11.0), 13.1);

    let report = runtime
        .next_technical_readiness_report(&session)
        .expect("active recovery failure should publish its recovery state");
    if runtime
        .snapshot()
        .recovery_episode
        .is_some_and(|episode| episode.degraded)
    {
        assert_eq!(report.phase, TechnicalPlayabilityPhase::TerminallyBlocked);
        assert_eq!(report.reason, Some(TechnicalBlockCause::RecoveryExhausted));
    } else {
        assert_eq!(report.phase, TechnicalPlayabilityPhase::TemporarilyBlocked);
        assert_eq!(report.reason, Some(TechnicalBlockCause::Recovery));
        assert_eq!(report.recovery, Some(RecoveryStage::Retrying));
    }
}
