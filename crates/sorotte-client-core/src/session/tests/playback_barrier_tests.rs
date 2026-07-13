use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use sorotte_protocol::{
    CommitStartPayload, PlaybackBarrierParticipantStatus, PlaybackBarrierPhase,
    PlaybackBarrierPolicy, PlaybackBarrierSetExtension, PlaybackBarrierStatusPayload,
    PlaystatePayload, PrepareMediaPayload, ProtocolMessage, RoomBufferingPhase,
    RoomBufferingPolicy, RoomBufferingPolicyPayload, RoomBufferingStatusPayload,
    SOROTTE_PLAYBACK_BARRIER_V1, SetPayload, StatePayload,
};

use super::*;

fn barrier_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("barrier-aware hello should apply");
    session
}

fn prepare(media_generation: u64) -> PrepareMediaPayload {
    PrepareMediaPayload::new(
        media_generation,
        format!("logical-media-{media_generation}"),
        12.5,
        PlaybackBarrierPolicy::AllEligible,
    )
    .with_request_nonce(media_generation.saturating_add(1))
    .with_timeout_ms(10_000)
    .with_deadline(110.0)
}

fn commit(media_generation: u64, state_revision: u64) -> CommitStartPayload {
    CommitStartPayload::new(media_generation, state_revision, 12.5, 100.0, 115.0)
}

fn status(
    media_generation: u64,
    state_revision: Option<u64>,
    phase: PlaybackBarrierPhase,
) -> PlaybackBarrierStatusPayload {
    PlaybackBarrierStatusPayload {
        media_generation,
        state_revision,
        phase,
        policy: PlaybackBarrierPolicy::AllEligible,
        quorum: None,
        deadline: 115.0,
        participants: BTreeMap::<String, PlaybackBarrierParticipantStatus>::new(),
        excluded_legacy_clients: BTreeSet::new(),
    }
}

fn apply_extension(session: &mut ClientSession, extension: PlaybackBarrierSetExtension) {
    session
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(extension),
        ))
        .expect("playback barrier extension should apply");
}

fn buffering_policy(
    media_generation: u64,
    state_revision: Option<u64>,
    policy: RoomBufferingPolicy,
) -> RoomBufferingPolicyPayload {
    let mut config = RoomBufferingPolicyPayload::new(media_generation, policy)
        .with_debounce_ms(750)
        .with_resume_hysteresis_ms(1_500)
        .with_max_pause_ms(30_000);
    if let Some(state_revision) = state_revision {
        config = config.with_state_revision(state_revision);
    }
    if policy == RoomBufferingPolicy::Quorum {
        config = config.with_quorum_percent(75);
    }
    config
}

fn buffering_status(
    config: RoomBufferingPolicyPayload,
    phase: RoomBufferingPhase,
) -> RoomBufferingStatusPayload {
    RoomBufferingStatusPayload {
        config,
        phase,
        eligible_clients: 2,
        required_buffering_clients: 1,
        buffering_clients: BTreeSet::new(),
        pause_deadline: None,
    }
}

#[test]
fn hello_feature_advertisement_is_explicit_and_server_negotiated() {
    let mut features = Map::from_iter([("chat".to_owned(), Value::Bool(true))]);
    ClientSession::advertise_playback_barrier_v1(&mut features);

    assert_eq!(
        features.get(SOROTTE_PLAYBACK_BARRIER_V1),
        Some(&Value::Bool(true))
    );
    assert_eq!(features.get("chat"), Some(&Value::Bool(true)));

    let aware = barrier_session();
    assert!(aware.playback_barrier_v1_negotiated());
    assert!(
        aware
            .server_capabilities()
            .is_some_and(|capabilities| capabilities.playback_barrier_v1)
    );

    let mut legacy = ClientSession::default();
    legacy
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("legacy hello should apply");
    assert!(!legacy.playback_barrier_v1_negotiated());
}

#[test]
fn prepare_allows_only_active_generation_media_ready_without_touching_user_readiness() {
    let mut session = barrier_session();
    session
        .apply_message_json(r#"{"Set":{"ready":{"username":"alice","isReady":true}}}"#)
        .expect("user readiness should apply");
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(7))
            .with_status(status(7, None, PlaybackBarrierPhase::Preparing)),
    );

    assert_eq!(
        session
            .playback_barrier_prepare()
            .map(|prepare| prepare.media_generation),
        Some(7)
    );
    assert!(
        session
            .playback_barrier_media_ready_observation(6, true, Some(true), true)
            .is_none(),
        "stale player generations must not report transport readiness"
    );

    let state = session
        .playback_barrier_media_ready_observation(7, true, Some(true), true)
        .expect("active generation should report transport readiness");
    assert!(state.playstate.is_none());
    let ready = state
        .playback_barrier_v1()
        .expect("outbound extension should decode")
        .and_then(|extension| extension.ready)
        .expect("outbound extension should contain MediaReady");
    assert_eq!(ready.media_generation, 7);
    assert!(ready.loaded);
    assert_eq!(ready.seekable, Some(true));
    assert!(ready.buffer_ready);
    assert_eq!(session.user_ready("alice"), Some(true));
}

#[test]
fn commit_is_retained_across_status_updates_and_rejects_revision_regression() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(4)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(commit(4, 9))
            .with_status(status(4, Some(9), PlaybackBarrierPhase::Committed)),
    );
    assert_eq!(
        session
            .playback_barrier_active_commit()
            .map(|commit| commit.state_revision),
        Some(9)
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(status(
            4,
            Some(9),
            PlaybackBarrierPhase::Complete,
        )),
    );
    assert_eq!(
        session
            .playback_barrier_commit()
            .map(|commit| commit.state_revision),
        Some(9),
        "status-only fanout must not discard CommitStart"
    );
    assert_eq!(
        session.playback_barrier_status().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete)
    );
    assert!(
        session.playback_barrier_active_commit().is_none(),
        "a retained terminal commit is diagnostic history, not playback authority"
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(commit(4, 8))
            .with_status(status(4, Some(8), PlaybackBarrierPhase::Committed)),
    );
    assert_eq!(
        session
            .playback_barrier_commit()
            .map(|commit| commit.state_revision),
        Some(9)
    );
    assert_eq!(
        session.playback_barrier_status().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete),
        "an older status must not regress the retained barrier"
    );
    assert!(
        session
            .playback_barrier_media_ready_observation(4, true, Some(true), true)
            .is_none(),
        "MediaReady is a prepare-phase observation"
    );
}

#[test]
fn terminal_barrier_phases_reject_delayed_awaiting_decision_updates() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(7))
            .with_status(status(7, None, PlaybackBarrierPhase::AwaitingDecision)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(status(
            7,
            None,
            PlaybackBarrierPhase::Degraded,
        )),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(status(
            7,
            None,
            PlaybackBarrierPhase::AwaitingDecision,
        )),
    );
    assert_eq!(
        session.playback_barrier_status().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Degraded),
        "a delayed decision-phase update must not revive a degraded lifecycle"
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(8))
            .with_commit(commit(8, 12))
            .with_status(status(8, Some(12), PlaybackBarrierPhase::Complete)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(status(
            8,
            Some(12),
            PlaybackBarrierPhase::AwaitingDecision,
        )),
    );
    assert_eq!(
        session.playback_barrier_status().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete),
        "a delayed decision-phase update must not replace completion"
    );
}

#[test]
fn degraded_barrier_deactivates_retained_commit_before_ordinary_pause() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(6)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(commit(6, 10))
            .with_status(status(6, Some(10), PlaybackBarrierPhase::Committed)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_status(status(
            6,
            Some(10),
            PlaybackBarrierPhase::Degraded,
        )),
    );
    session
        .apply_protocol_message(ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(14.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("bob"),
            ),
        ))
        .expect("ordinary pause should apply after terminal barrier status");

    assert!(session.playback_barrier_commit().is_some());
    assert!(session.playback_barrier_active_commit().is_none());
    assert_eq!(
        session
            .current_room_playstate()
            .and_then(|playstate| playstate.paused),
        Some(true)
    );
}

#[test]
fn started_ack_requires_actual_advancement_and_exact_commit_identity() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(12)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(commit(12, 3))
            .with_status(status(12, Some(3), PlaybackBarrierPhase::Committed)),
    );

    assert!(
        session
            .playback_barrier_started_observation(12, 3, 12.6, false, Some(101.0))
            .is_none(),
        "command acceptance without observed advancement is not a start"
    );
    assert!(
        session
            .playback_barrier_started_observation(11, 3, 12.6, true, Some(101.0))
            .is_none()
    );
    assert!(
        session
            .playback_barrier_started_observation(12, 2, 12.6, true, Some(101.0))
            .is_none()
    );
    assert!(
        session
            .playback_barrier_started_observation(12, 3, f64::NAN, true, Some(101.0))
            .is_none()
    );

    let state = session
        .playback_barrier_started_observation(12, 3, 12.6, true, Some(101.0))
        .expect("matching observed advancement should acknowledge actual start");
    let started = state
        .playback_barrier_v1()
        .expect("outbound extension should decode")
        .and_then(|extension| extension.started)
        .expect("outbound extension should contain StartedAck");
    assert_eq!(started.media_generation, 12);
    assert_eq!(started.state_revision, 3);
    assert_eq!(started.observed_position, 12.6);
    assert_eq!(started.observed_at, Some(101.0));
}

#[test]
fn stale_generations_and_observations_cannot_mutate_the_active_barrier() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(20)),
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_commit(commit(20, 6))
            .with_status(status(20, Some(6), PlaybackBarrierPhase::Committed)),
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(19))
            .with_commit(commit(19, 99))
            .with_status(status(19, Some(99), PlaybackBarrierPhase::Complete)),
    );
    assert_eq!(
        session
            .playback_barrier_prepare()
            .map(|prepare| prepare.media_generation),
        Some(20)
    );
    assert_eq!(
        session
            .playback_barrier_commit()
            .map(|commit| commit.state_revision),
        Some(6)
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(21)),
    );
    assert!(session.playback_barrier_commit().is_none());
    assert!(session.playback_barrier_status().is_none());
    assert!(
        session
            .playback_barrier_started_observation(20, 6, 13.0, true, None)
            .is_none(),
        "an observed start from the superseded media must be ignored"
    );
}

#[test]
fn runtime_reports_only_valid_barrier_observations_as_state_extensions() {
    let mut session = barrier_session();
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(24))
            .with_status(status(24, None, PlaybackBarrierPhase::Preparing)),
    );
    let mut runtime = ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );

    assert!(
        !runtime
            .report_playback_barrier_media_ready(23, true, Some(true), true)
            .expect("stale readiness should be rejected without a sink failure")
    );
    assert!(runtime.flush_queued_protocol_messages().is_empty());
    assert!(
        runtime
            .report_playback_barrier_media_ready(24, true, Some(true), true)
            .expect("active readiness should queue")
    );
    let ready_messages = runtime.flush_queued_protocol_messages();
    assert_eq!(ready_messages.len(), 1);
    let ProtocolMessage::State(ready_message) = &ready_messages[0] else {
        panic!("MediaReady should use the State extension");
    };
    assert_eq!(
        ready_message
            .state
            .playback_barrier_v1()
            .expect("ready extension should decode")
            .and_then(|extension| extension.ready)
            .map(|ready| ready.media_generation),
        Some(24)
    );

    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_commit(commit(24, 7))
                    .with_status(status(24, Some(7), PlaybackBarrierPhase::Committed)),
            ),
        ))
        .expect("commit should apply through ClientSessionUpdate");
    assert!(
        !runtime
            .report_playback_barrier_started(24, 7, 12.7, false, Some(102.0))
            .expect("non-advancing observation should be rejected")
    );
    assert!(
        !runtime
            .report_playback_barrier_started(24, 6, 12.7, true, Some(102.0))
            .expect("stale revision should be rejected")
    );
    assert!(runtime.flush_queued_protocol_messages().is_empty());
    assert!(
        runtime
            .report_playback_barrier_started(24, 7, 12.7, true, Some(102.0))
            .expect("matching actual start should queue")
    );
    let started_messages = runtime.flush_queued_protocol_messages();
    assert_eq!(started_messages.len(), 1);
    let ProtocolMessage::State(started_message) = &started_messages[0] else {
        panic!("StartedAck should use the State extension");
    };
    assert_eq!(
        started_message
            .state
            .playback_barrier_v1()
            .expect("started extension should decode")
            .and_then(|extension| extension.started)
            .map(|started| (started.media_generation, started.state_revision)),
        Some((24, 7))
    );
}

#[test]
fn barrier_extension_and_ordinary_playstate_remain_separate_in_mixed_rooms() {
    let mut session = barrier_session();
    let mut preparing_status = status(31, None, PlaybackBarrierPhase::Preparing);
    preparing_status
        .excluded_legacy_clients
        .insert("legacy-bob".to_owned());
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_prepare(prepare(31))
            .with_status(preparing_status),
    );
    session
        .apply_protocol_message(ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(12.5)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ))
        .expect("ordinary room playstate should still apply");

    assert_eq!(
        session
            .current_room_playstate()
            .and_then(|playstate| playstate.position),
        Some(12.5)
    );
    assert_eq!(
        session
            .playback_barrier_prepare()
            .map(|prepare| prepare.media_generation),
        Some(31)
    );
    assert!(
        session
            .playback_barrier_status()
            .is_some_and(|status| status.excluded_legacy_clients.contains("legacy-bob"))
    );
    assert!(session.drain_compatibility_fallbacks().is_empty());

    let mut legacy_session = ClientSession::default();
    legacy_session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("legacy hello should apply");
    apply_extension(
        &mut legacy_session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(31)),
    );
    legacy_session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":18.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("legacy ordinary playstate should apply");
    assert!(legacy_session.playback_barrier_prepare().is_none());
    assert_eq!(
        legacy_session
            .current_room_playstate()
            .and_then(|playstate| playstate.position),
        Some(18.0)
    );
}

#[test]
fn ongoing_buffering_policy_and_status_are_validated_and_survive_prepare_resets() {
    let mut session = barrier_session();
    let active_policy = buffering_policy(40, Some(5), RoomBufferingPolicy::PauseAnyEligible);
    let active_status = buffering_status(active_policy.clone(), RoomBufferingPhase::Monitoring);
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new()
            .with_buffering_policy(active_policy.clone())
            .with_buffering_status(active_status.clone()),
    );
    assert_eq!(
        session.playback_barrier_buffering_policy(),
        Some(&active_policy)
    );
    assert_eq!(
        session.playback_barrier_buffering_status(),
        Some(&active_status)
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_prepare(prepare(41)),
    );
    assert_eq!(
        session.playback_barrier_buffering_policy(),
        Some(&active_policy),
        "a start-barrier prepare reset must not discard ongoing policy state"
    );
    assert_eq!(
        session.playback_barrier_buffering_status(),
        Some(&active_status)
    );
    assert!(
        session
            .playback_barrier_transport_observation(40, Some(5), true, Some(0.0), Some(100.0),)
            .is_none(),
        "retained policy is queryable but cannot report for media superseded by a newer prepare"
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(buffering_policy(
            39,
            Some(99),
            RoomBufferingPolicy::PauseController,
        )),
    );
    let invalid_quorum = RoomBufferingPolicyPayload::new(42, RoomBufferingPolicy::Quorum)
        .with_debounce_ms(500)
        .with_resume_hysteresis_ms(500)
        .with_max_pause_ms(5_000);
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(invalid_quorum),
    );
    assert_eq!(
        session.playback_barrier_buffering_policy(),
        Some(&active_policy),
        "older and structurally invalid policies must not replace active state"
    );

    let next_policy = buffering_policy(42, None, RoomBufferingPolicy::PauseController);
    let next_status = buffering_status(next_policy.clone(), RoomBufferingPhase::Paused);
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_status(next_status.clone()),
    );
    assert_eq!(
        session.playback_barrier_buffering_policy(),
        Some(&next_policy),
        "status.config is authoritative when a separate policy echo is absent"
    );
    assert_eq!(
        session.playback_barrier_buffering_status(),
        Some(&next_status)
    );

    let mut invalid_status = next_status.clone();
    invalid_status.required_buffering_clients = 3;
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_status(invalid_status),
    );
    assert_eq!(
        session.playback_barrier_buffering_status(),
        Some(&next_status),
        "an impossible eligible-cohort projection must be ignored"
    );
}

#[test]
fn transport_observation_requires_exact_policy_identity_and_deduplicates() {
    let mut session = barrier_session();
    let active_policy = buffering_policy(50, Some(8), RoomBufferingPolicy::PauseAnyEligible);
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(active_policy.clone()),
    );

    assert!(
        session
            .playback_barrier_transport_observation(49, Some(8), true, Some(0.25), Some(100.0),)
            .is_none()
    );
    assert!(
        session
            .playback_barrier_transport_observation(50, Some(7), true, Some(0.25), Some(100.0),)
            .is_none()
    );
    assert!(
        session
            .playback_barrier_transport_observation(50, Some(8), true, Some(f64::NAN), Some(100.0),)
            .is_none()
    );

    let state = session
        .playback_barrier_transport_observation(50, Some(8), true, Some(0.25), Some(100.0))
        .expect("first exact observation should be built");
    let transport = state
        .playback_barrier_v1()
        .expect("transport extension should decode")
        .and_then(|extension| extension.transport)
        .expect("transport observation should be present");
    assert_eq!(transport.media_generation, 50);
    assert_eq!(transport.state_revision, Some(8));
    assert!(transport.buffering);
    assert_eq!(transport.buffered_seconds, Some(0.25));
    assert_eq!(transport.observed_at, Some(100.0));

    assert!(
        session
            .playback_barrier_transport_observation(50, Some(8), true, Some(0.25), Some(100.0),)
            .is_none(),
        "an exact duplicate must not enqueue another State obligation"
    );
    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(active_policy),
    );
    assert!(
        session
            .playback_barrier_transport_observation(50, Some(8), true, Some(0.25), Some(100.0),)
            .is_some(),
        "an authoritative policy snapshot must rearm the current transport report"
    );
    assert!(
        session
            .playback_barrier_transport_observation(50, Some(8), false, Some(2.0), Some(101.0),)
            .is_some(),
        "a recovery transition must not be deduplicated"
    );

    apply_extension(
        &mut session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(buffering_policy(
            50,
            Some(8),
            RoomBufferingPolicy::PauseController,
        )),
    );
    assert!(
        session
            .playback_barrier_transport_observation(50, Some(8), false, Some(2.0), Some(101.0),)
            .is_some(),
        "a policy replacement resets observation deduplication"
    );
}
