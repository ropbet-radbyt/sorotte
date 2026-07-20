use super::*;
use crate::{PendingUserTransportEvidence, READINESS_USER_TRANSPORT_GRACE_SECONDS};
use sorotte_client_app::app_boundary::{
    application::{ClientApplication, ClientApplicationSettings, ClientCommand},
    state::{
        ClientConfig, StartSynchronizationConfig, StartSynchronizationPolicy,
        StoredClientSettingsV1,
    },
};
use sorotte_client_core::{
    ClientSession, LogicalMediaId, MediaTransportKind, PlaybackBarrierStartConfig,
};
use sorotte_player_api::DisconnectedPlayer;
use sorotte_protocol::{
    DirectReadinessSurface, MediaReadyPayload, MixedReadinessPolicy, PlaybackBarrierDegradedReason,
    PlaybackBarrierParticipantPhase, PlaybackBarrierPhase, PlaybackBarrierPolicy,
    PlaybackBarrierSetExtension, PlaybackBarrierStateExtension, PlayerInteractionSurface,
    PlayerReadinessAction, ReadinessIntentRequest, ReadinessMutationSource,
    ReadinessRequestResultPayload, ReadinessRequestResultStatus, ReadinessSetExtension,
    ReadinessStateExtension, RecoveryStage, RoomPauseOwner, RoomReadinessSnapshot,
    RoomStartGatePhase, SOROTTE_PLAYBACK_BARRIER_V1, SOROTTE_READINESS_RECONNECT_TOKEN,
    SOROTTE_READINESS_V2, StartGateDegradedReason, StartParticipationRole, StartedAckPayload,
    StatePayload, TechnicalBlockCause, TechnicalPlayability, TechnicalPlayabilityPhase,
    TechnicalReadinessReport, UserReadinessIntent, UserReadinessMutationSource,
    encode_message_line,
};
use std::collections::BTreeMap;

const READINESS_CAPABILITIES: &str = r#""sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true"#;

fn readiness_hello(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{{READINESS_CAPABILITIES}}}}}}}"#
    )
}

fn readiness_hello_with_token(username: &str, room: &str, token: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{{READINESS_CAPABILITIES}}},"{SOROTTE_READINESS_RECONNECT_TOKEN}":"{token}"}}}}"#
    )
}

fn reconnect_token_for(lines: &[DirectedOutboundLine], recipient: &str) -> String {
    decode_directed_lines(lines)
        .into_iter()
        .find_map(|(client_id, message)| {
            if client_id != recipient {
                return None;
            }
            let ProtocolMessage::Hello(hello) = message else {
                return None;
            };
            hello
                .hello
                .extra
                .get(SOROTTE_READINESS_RECONNECT_TOKEN)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .expect("readiness Hello should issue a reconnect token")
}

fn legacy_hello(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5"}}}}"#
    )
}

fn playback_barrier_hello(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"sorottePlaybackBarrierV1":true}}}}}}"#
    )
}

fn readiness_only_hello(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"sorotteReadinessV2":true}}}}}}"#
    )
}

fn readiness_extension(message: &ProtocolMessage) -> Option<ReadinessSetExtension> {
    let ProtocolMessage::Set(set) = message else {
        return None;
    };
    set.set.readiness_v2().ok().flatten()
}

fn barrier_extension(message: &ProtocolMessage) -> Option<PlaybackBarrierSetExtension> {
    let ProtocolMessage::Set(set) = message else {
        return None;
    };
    set.set.playback_barrier_v1().ok().flatten()
}

fn readiness_snapshot_for(
    lines: &[DirectedOutboundLine],
    recipient: &str,
) -> Option<RoomReadinessSnapshot> {
    decode_directed_lines(lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == recipient)
        .filter_map(|(_, message)| readiness_extension(&message))
        .find_map(|extension| extension.snapshot)
}

fn readiness_result_for(
    lines: &[DirectedOutboundLine],
    recipient: &str,
) -> Option<ReadinessRequestResultPayload> {
    decode_directed_lines(lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == recipient)
        .filter_map(|(_, message)| readiness_extension(&message))
        .find_map(|extension| extension.request_result)
}

fn send_intent(
    runtime: &mut ServerRuntime,
    client_id: &str,
    operation_id: &str,
    request_nonce: u64,
    membership_epoch: u64,
    desired: UserReadinessIntent,
) -> Vec<DirectedOutboundLine> {
    send_intent_with_source(
        runtime,
        client_id,
        operation_id,
        request_nonce,
        membership_epoch,
        desired,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn send_intent_with_source(
    runtime: &mut ServerRuntime,
    client_id: &str,
    operation_id: &str,
    request_nonce: u64,
    membership_epoch: u64,
    desired: UserReadinessIntent,
    source: UserReadinessMutationSource,
) -> Vec<DirectedOutboundLine> {
    let request = ReadinessIntentRequest::new(
        operation_id,
        request_nonce,
        membership_epoch,
        desired,
        source,
    );
    let message = ProtocolMessage::set(
        SetPayload::new().with_readiness_v2(ReadinessSetExtension::new().with_intent(request)),
    );
    let line = encode_message_line(&message).expect("readiness intent should encode");
    runtime
        .handle_line_fanout(client_id, &line)
        .expect("readiness intent should be accepted as a protocol command")
}

fn send_barrier_ready(
    runtime: &mut ServerRuntime,
    client_id: &str,
    media_generation: u64,
    ready: bool,
) -> Vec<DirectedOutboundLine> {
    send_barrier_observation(
        runtime,
        client_id,
        MediaReadyPayload::new(media_generation, ready, ready),
    )
}

fn send_barrier_observation(
    runtime: &mut ServerRuntime,
    client_id: &str,
    observation: MediaReadyPayload,
) -> Vec<DirectedOutboundLine> {
    let message = ProtocolMessage::state(
        StatePayload::new()
            .with_playback_barrier_v1(PlaybackBarrierStateExtension::new().with_ready(observation)),
    );
    let line = encode_message_line(&message).expect("barrier readiness should encode");
    runtime
        .handle_line_fanout(client_id, &line)
        .expect("barrier readiness should be accepted as a protocol command")
}

fn send_technical(
    runtime: &mut ServerRuntime,
    client_id: &str,
    mut report: TechnicalReadinessReport,
) -> Vec<DirectedOutboundLine> {
    let session = &runtime.sessions[client_id];
    let participant = &runtime.room_readiness[&session.room].participants[&session.username];
    report.membership_epoch = participant.record.membership_epoch;
    report.report_sequence = participant
        .record
        .last_technical_report_sequence
        .saturating_add(1);
    if report.authoritative_playback_revision.is_none() {
        report.authoritative_playback_revision =
            if report.reason == Some(TechnicalBlockCause::RoomBufferingPolicy) {
                runtime
                    .room_buffering_controls
                    .get(&session.room)
                    .and_then(|control| control.config.state_revision)
                    .or_else(|| {
                        runtime
                            .room_playback_barriers
                            .get(&session.room)
                            .and_then(|barrier| barrier.state_revision)
                    })
            } else {
                runtime
                    .room_playback_barriers
                    .get(&session.room)
                    .filter(|barrier| barrier.commit.is_some())
                    .and_then(|barrier| barrier.state_revision)
            };
    }
    send_technical_exact(runtime, client_id, report)
}

fn send_technical_exact(
    runtime: &mut ServerRuntime,
    client_id: &str,
    report: TechnicalReadinessReport,
) -> Vec<DirectedOutboundLine> {
    let message = ProtocolMessage::state(
        StatePayload::new()
            .with_readiness_v2(ReadinessStateExtension::new().with_technical(report)),
    );
    let line = encode_message_line(&message).expect("technical readiness should encode");
    runtime
        .handle_line_fanout(client_id, &line)
        .expect("technical readiness should be accepted as a protocol command")
}

fn unscoped_technical_report(
    media_generation: u64,
    phase: TechnicalPlayabilityPhase,
) -> TechnicalReadinessReport {
    TechnicalReadinessReport::new(media_generation, 1, 1, phase)
}

fn acknowledge_forced_state(runtime: &mut ServerRuntime, client_id: &str) {
    let counter = runtime.server_ignoring_counter(client_id);
    runtime.acknowledge_server_ignoring_counter(client_id, counter);
}

fn send_playstate(
    runtime: &mut ServerRuntime,
    client_id: &str,
    position: f64,
    paused: bool,
) -> Vec<DirectedOutboundLine> {
    runtime
        .handle_line_fanout(
            client_id,
            &format!(
                r#"{{"State":{{"playstate":{{"position":{position},"paused":{paused},"doSeek":false}}}}}}"#
            ),
        )
        .expect("playstate should be accepted as a protocol command")
}

fn send_started(
    runtime: &mut ServerRuntime,
    client_id: &str,
    media_generation: u64,
    state_revision: u64,
    observed_position: f64,
) -> Vec<DirectedOutboundLine> {
    let message = ProtocolMessage::state(StatePayload::new().with_playback_barrier_v1(
        PlaybackBarrierStateExtension::new().with_started(StartedAckPayload::new(
            media_generation,
            state_revision,
            observed_position,
        )),
    ));
    let line = encode_message_line(&message).expect("Started acknowledgement should encode");
    runtime
        .handle_line_fanout(client_id, &line)
        .expect("Started acknowledgement should be accepted as a protocol command")
}

fn start_barrier(runtime: &mut ServerRuntime, client_id: &str) -> Vec<DirectedOutboundLine> {
    runtime
        .handle_line_fanout(
            client_id,
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"readiness:test-media","targetPosition":12.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
        )
        .expect("playback barrier should start")
}

fn awaiting_controller_decision_runtime(
    room_name: &str,
    canonical_paused: bool,
) -> (ServerRuntime, u64) {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", room_name))
        .expect("readiness hello should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"readiness:ask-controller","targetPosition":0.0,"policy":"allEligible","timeoutMs":1000,"timeoutAction":"askController"}}}}"#,
        )
        .expect("AskController barrier should start");
    runtime
        .collect_dispatch_at(101.0)
        .expect("AskController barrier should time out");
    assert_eq!(
        runtime.room_playback_barriers[room_name].phase,
        PlaybackBarrierPhase::AwaitingDecision
    );
    if !canonical_paused {
        runtime.room_playback_state_mut(room_name).paused = false;
        runtime
            .room_readiness
            .get_mut(room_name)
            .unwrap()
            .pause_owner = RoomPauseOwner::None;
    }
    acknowledge_forced_state(&mut runtime, "alice-client");
    let epoch = runtime.room_readiness[room_name].participants["alice"]
        .record
        .membership_epoch;
    (runtime, epoch)
}

#[test]
fn readiness_hello_advertises_v2_and_initializes_fresh_membership_not_ready() {
    let mut runtime = ServerRuntime::default();
    let lines = runtime
        .handle_line_fanout("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let messages = decode_directed_lines(&lines);
    let server_hello = messages
        .iter()
        .find_map(|(_, message)| match message {
            ProtocolMessage::Hello(hello) => Some(&hello.hello),
            _ => None,
        })
        .expect("server hello should be published");
    assert_eq!(
        server_hello
            .features
            .as_ref()
            .and_then(|features| features.get(SOROTTE_READINESS_V2))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        server_hello
            .features
            .as_ref()
            .and_then(|features| features.get(SOROTTE_PLAYBACK_BARRIER_V1))
            .and_then(Value::as_bool),
        Some(true)
    );

    let snapshot = readiness_snapshot_for(&lines, "alice-client")
        .expect("fresh membership should publish a room snapshot");
    let alice = &snapshot.participants["alice"];
    assert!(alice.membership_epoch > 0);
    assert_eq!(alice.user_intent, UserReadinessIntent::NotReady);
    assert_eq!(alice.participation_role, StartParticipationRole::Required);
    assert!(!alice.room_ready);
    assert!(!alice.start_eligible);
    assert_eq!(snapshot.pause_owner, RoomPauseOwner::None);
}

#[test]
fn client_initialization_is_self_scoped_pristine_idempotent_and_projected_as_system() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("alice hello should succeed");
    runtime
        .handle_line("bob-client", &readiness_hello("bob", "room"))
        .expect("bob hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;

    let false_initialization = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "ready-at-start-false",
        1,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&false_initialization, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedInvalid),
        "NotReady is already the fresh default and must not consume initialization"
    );

    let initialized = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "ready-at-start",
        1,
        epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&initialized, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Accepted)
    );
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert_eq!(alice.user_intent_revision, 1);
    assert!(matches!(
        alice
            .last_user_mutation
            .as_ref()
            .map(|mutation| &mutation.source),
        Some(ReadinessMutationSource::Initialization)
    ));
    assert!(
        decode_directed_lines(&initialized)
            .iter()
            .any(|(_, message)| {
                matches!(
                    message,
                    ProtocolMessage::Set(set)
                        if set.set.ready.as_ref().is_some_and(|ready| {
                            ready.username.as_deref() == Some("alice")
                                && ready.is_ready == Some(true)
                                && ready.manually_initiated == Some(false)
                        })
                )
            })
    );

    let duplicate = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "ready-at-start",
        50,
        epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&duplicate, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Duplicate)
    );
    let later_initialization = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "late-initialization",
        2,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&later_initialization, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedInvalid)
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );

    let target_request = ReadinessIntentRequest::new(
        "targeted-initialization",
        2,
        epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::Initialization,
    )
    .with_target_username("bob");
    let target_message = ProtocolMessage::set(
        SetPayload::new()
            .with_readiness_v2(ReadinessSetExtension::new().with_intent(target_request)),
    );
    let targeted = runtime
        .handle_line_fanout(
            "alice-client",
            &encode_message_line(&target_message).expect("target request should encode"),
        )
        .expect("target request should decode");
    assert_eq!(
        readiness_result_for(&targeted, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedInvalid)
    );

    let mut reconnected = ServerRuntime::default();
    let initial = reconnected
        .handle_line_fanout(
            "fresh-old-client",
            &readiness_hello("fresh", "reconnect-room"),
        )
        .expect("fresh hello should succeed");
    let reconnect_token = reconnect_token_for(&initial, "fresh-old-client");
    let fresh_epoch = reconnected.room_readiness["reconnect-room"].participants["fresh"]
        .record
        .membership_epoch;
    reconnected
        .handle_transport_disconnect_fanout("fresh-old-client")
        .expect("disconnect should succeed");
    reconnected
        .handle_line(
            "fresh-new-client",
            &readiness_hello_with_token("fresh", "reconnect-room", &reconnect_token),
        )
        .expect("reconnect should restore membership");
    let reconnect_initialization = send_intent_with_source(
        &mut reconnected,
        "fresh-new-client",
        "late-ready-at-start",
        1,
        fresh_epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&reconnect_initialization, "fresh-new-client")
            .map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedInvalid),
        "initialization is only open on the first attachment of a fresh membership"
    );
}

#[test]
fn strict_mixed_room_policy_blocks_automatic_start_and_explains_legacy_incompatibility() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    runtime
        .handle_line("legacy-client", &legacy_hello("legacy", "room"))
        .expect("legacy hello should succeed");
    let alice_epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-strict-ready",
        1,
        alice_epoch,
        UserReadinessIntent::Ready,
    );

    let lines = start_barrier(&mut runtime, "alice-client");
    let status = decode_directed_lines(&lines)
        .into_iter()
        .filter(|(client_id, _)| client_id == "alice-client")
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("capable client should receive the canonical cohort policy");
    assert_eq!(
        status.excluded_legacy_clients,
        BTreeSet::from(["legacy".to_owned()]),
        "legacy exclusion must be explicit rather than inferred from absence"
    );
    let snapshot = readiness_snapshot_for(&lines, "alice-client")
        .expect("capable client should receive strict mixed-room gate state");
    assert_eq!(
        snapshot.mixed_readiness_policy,
        MixedReadinessPolicy::RequireAllMembers
    );
    assert_eq!(
        snapshot.start_gate_phase,
        RoomStartGatePhase::Degraded {
            media_generation: 1,
            reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
        }
    );
    assert!(!runtime.playback_barrier_policy_satisfied("room"));
    assert!(
        decode_directed_lines(&lines)
            .iter()
            .all(|(client_id, message)| {
                client_id != "legacy-client" || readiness_extension(message).is_none()
            }),
        "legacy clients must not receive an extension they did not advertise"
    );

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert!(matches!(
        alice.technical_state,
        TechnicalPlayability::Playable {
            media_generation: 1
        }
    ));
    assert!(alice.room_ready);
    assert!(alice.start_eligible);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "the fully eligible V2 participant cannot make strict mixed-room policy ignore legacy"
    );
    assert_eq!(runtime.next_playback_barrier_revision, 0);
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Degraded {
            media_generation: 1,
            reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
        },
        "the legacy participant must be the sole remaining blocker"
    );
}

#[test]
fn legacy_self_ready_omits_set_by_while_controller_and_mixed_updates_derive_actor() {
    let ready_updates = |lines: &[DirectedOutboundLine], username: &str| {
        decode_directed_lines(lines)
            .into_iter()
            .filter_map(|(_, message)| match message {
                ProtocolMessage::Set(set) => set.set.ready,
                _ => None,
            })
            .filter(|ready| ready.username.as_deref() == Some(username))
            .collect::<Vec<_>>()
    };

    let mut legacy_only = ServerRuntime::default();
    legacy_only
        .handle_line("alice-client", &legacy_hello("alice", "legacy-room"))
        .expect("legacy alice should join");
    legacy_only
        .handle_line("bob-client", &legacy_hello("bob", "legacy-room"))
        .expect("legacy bob should join");
    let self_ready = legacy_only
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("legacy self Ready should succeed");
    let self_updates = ready_updates(&self_ready, "alice");
    assert!(!self_updates.is_empty());
    assert!(self_updates.iter().all(|ready| ready.set_by.is_none()));

    let controller_ready = legacy_only
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"ready":{"username":"bob","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("legacy controller Ready should succeed");
    let controller_updates = ready_updates(&controller_ready, "bob");
    assert!(!controller_updates.is_empty());
    assert!(
        controller_updates
            .iter()
            .all(|ready| ready.set_by.as_deref() == Some("alice"))
    );

    let mut mixed = ServerRuntime::default();
    mixed
        .handle_line("v2-client", &readiness_hello("v2", "mixed-room"))
        .expect("V2 member should join");
    mixed
        .handle_line("legacy-client", &legacy_hello("legacy", "mixed-room"))
        .expect("legacy member should join");
    let mixed_ready = mixed
        .handle_line_fanout(
            "v2-client",
            r#"{"Set":{"ready":{"username":"legacy","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("V2 controller should set legacy readiness");
    let mixed_updates = ready_updates(&mixed_ready, "legacy");
    assert!(!mixed_updates.is_empty());
    assert!(
        mixed_updates
            .iter()
            .all(|ready| ready.set_by.as_deref() == Some("v2"))
    );
}

#[test]
fn active_readiness_barrier_refreshes_legacy_exclusions_on_join_and_disconnect() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    start_barrier(&mut runtime, "alice-client");
    let generation = runtime.room_playback_barriers["room"]
        .prepare
        .media_generation;

    runtime
        .handle_line("legacy-client", &legacy_hello("legacy", "room"))
        .expect("legacy hello should refresh the active cohort");
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.phase, PlaybackBarrierPhase::Preparing);
    assert_eq!(barrier.prepare.media_generation, generation);
    assert_eq!(
        barrier.excluded_legacy_clients,
        BTreeSet::from(["legacy".to_owned()])
    );
    assert_eq!(
        barrier
            .participants
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["alice-client".to_owned()])
    );

    runtime
        .handle_transport_disconnect_fanout("legacy-client")
        .expect("legacy disconnect should refresh the active cohort");
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.phase, PlaybackBarrierPhase::Preparing);
    assert_eq!(barrier.prepare.media_generation, generation);
    assert!(barrier.excluded_legacy_clients.is_empty());
    assert_eq!(
        barrier
            .participants
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["alice-client".to_owned()])
    );
}

#[test]
fn legacy_ready_bridge_preserves_v2_fences_and_authenticated_controller_compatibility() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    for (client_id, hello) in [
        ("alice-client", readiness_hello("alice", &room)),
        ("bob-client", readiness_hello("bob", &room)),
        ("carol-client", legacy_hello("carol", &room)),
        ("dave-client", legacy_hello("dave", &room)),
    ] {
        runtime
            .handle_line(client_id, &hello)
            .expect("participant hello should succeed");
    }
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        let epoch = runtime.room_readiness[&room].participants[username]
            .record
            .membership_epoch;
        send_intent(
            &mut runtime,
            client_id,
            &format!("{username}-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
    }
    for controller in ["alice-client", "carol-client"] {
        runtime
            .handle_line(
                controller,
                r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
            )
            .expect("controller authentication should succeed");
    }

    let revision_before_legacy_v2_commands = runtime.room_readiness[&room].revision;
    let self_projection = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"ready":{"isReady":false,"manuallyInitiated":false}}}"#,
        )
        .expect("legacy self projection should be ignored safely");
    assert!(self_projection.is_empty());
    runtime
        .handle_line(
            "alice-client",
            r#"{"Set":{"ready":{"username":"bob","isReady":false,"manuallyInitiated":true}}}"#,
        )
        .expect("legacy V2-target command should be ignored safely");
    assert_eq!(
        runtime.room_readiness[&room].revision, revision_before_legacy_v2_commands,
        "raw legacy commands from a V2 actor must not bypass V2 operation fencing"
    );
    assert_eq!(
        runtime.room_readiness[&room].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );
    assert_eq!(
        runtime.room_readiness[&room].participants["bob"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );

    runtime
        .handle_line(
            "alice-client",
            r#"{"Set":{"ready":{"username":"dave","isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("V2 controller should retain legacy-target compatibility");
    assert_eq!(runtime.stored_user_ready("dave", &room), Some(true));

    runtime
        .handle_line(
            "carol-client",
            r#"{"Set":{"ready":{"username":"bob","isReady":false,"manuallyInitiated":true}}}"#,
        )
        .expect("legacy controller should bridge to a V2 target");
    let bob = &runtime.room_readiness[&room].participants["bob"].record;
    assert_eq!(bob.user_intent, UserReadinessIntent::NotReady);
    assert!(matches!(
        bob.last_user_mutation.as_ref().map(|mutation| &mutation.source),
        Some(ReadinessMutationSource::ControllerOverride { actor }) if actor == "carol"
    ));
}

#[test]
fn mixed_room_excludes_barrier_capable_legacy_client_from_v2_commit_cohort() {
    let mut runtime = ServerRuntime::default();
    runtime.set_mixed_readiness_policy(MixedReadinessPolicy::ExcludeLegacy);
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    runtime
        .handle_line("legacy-client", &playback_barrier_hello("legacy", "room"))
        .expect("legacy barrier hello should succeed");
    let alice_epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        alice_epoch,
        UserReadinessIntent::Ready,
    );

    let started = start_barrier(&mut runtime, "alice-client");
    let status = decode_directed_lines(&started)
        .into_iter()
        .filter(|(client_id, _)| client_id == "alice-client")
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("V2 participant should receive mixed-cohort status");
    assert_eq!(
        status.participants.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["alice".to_owned()])
    );
    assert_eq!(
        status.excluded_legacy_clients,
        BTreeSet::from(["legacy".to_owned()])
    );

    let legacy_ack = send_barrier_ready(&mut runtime, "legacy-client", 1, true);
    assert!(
        decode_directed_lines(&legacy_ack)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| extension.commit.is_none())),
        "an excluded legacy MediaReady cannot enter or commit the V2 cohort"
    );
    assert!(
        !runtime.room_playback_barriers["room"]
            .participants
            .contains_key("legacy-client")
    );

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "generic playability cannot manufacture strict barrier readiness"
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed,
        "the documented exclusion policy makes only the V2 cohort authoritative"
    );
}

#[test]
fn readiness_only_members_are_excluded_and_an_empty_required_cohort_cannot_commit() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("legacy-client", &playback_barrier_hello("legacy", "room"))
        .expect("legacy barrier hello should succeed");
    start_barrier(&mut runtime, "legacy-client");
    runtime
        .handle_line(
            "readiness-only-client",
            &readiness_only_hello("readiness-only", "room"),
        )
        .expect("readiness-only hello should succeed");

    let readiness_only = &runtime.room_readiness["room"].participants["readiness-only"].record;
    assert_eq!(
        readiness_only.participation_role,
        StartParticipationRole::ExcludedLegacy
    );
    assert!(!readiness_only.start_eligible);
    let barrier = &runtime.room_playback_barriers["room"];
    assert!(barrier.participants.is_empty());
    assert_eq!(
        barrier.excluded_legacy_clients,
        BTreeSet::from(["legacy".to_owned(), "readiness-only".to_owned()])
    );
    assert!(!runtime.playback_barrier_policy_satisfied("room"));

    send_barrier_ready(&mut runtime, "legacy-client", 1, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "a room with no Required V2 participant must fail closed"
    );
}

#[test]
fn feature_changes_reconcile_required_role_and_active_barrier_membership() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"features":{"sorotteReadinessV2":true}}}"#,
        )
        .expect("barrier capability downgrade should succeed");
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .participation_role,
        StartParticipationRole::ExcludedLegacy
    );
    assert!(
        runtime.room_playback_barriers["room"]
            .participants
            .is_empty()
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].excluded_legacy_clients,
        BTreeSet::from(["alice".to_owned()])
    );
    assert!(!runtime.playback_barrier_policy_satisfied("room"));

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"features":{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#,
        )
        .expect("barrier capability upgrade should succeed");
    let record = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(record.participation_role, StartParticipationRole::Required);
    assert_eq!(record.user_intent, UserReadinessIntent::Ready);
    assert!(!record.start_eligible);
    assert!(
        runtime.room_playback_barriers["room"]
            .participants
            .contains_key("alice-client")
    );
    assert!(
        runtime.room_playback_barriers["room"]
            .excluded_legacy_clients
            .is_empty()
    );

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
}

#[test]
fn live_legacy_to_v2_upgrade_preserves_acknowledged_ready_intent() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &playback_barrier_hello("alice", "room"))
        .expect("legacy-capable hello should succeed");
    runtime
        .handle_line(
            "alice-client",
            r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
        )
        .expect("legacy Ready intent should be acknowledged");
    assert_eq!(runtime.stored_user_ready("alice", "room"), Some(true));

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"features":{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#,
        )
        .expect("live readiness capability upgrade should succeed");
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert!(alice.room_ready);
    assert_eq!(alice.participation_role, StartParticipationRole::Required);
    assert!(matches!(
        alice
            .last_user_mutation
            .as_ref()
            .map(|mutation| &mutation.source),
        Some(ReadinessMutationSource::Initialization)
    ));
    let membership_epoch = alice.membership_epoch;
    let forged_initialization = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "upgrade-is-not-fresh-initialization",
        1,
        membership_epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::Initialization,
    );
    assert_eq!(
        readiness_result_for(&forged_initialization, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedInvalid)
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready,
        "legacy-projected Ready cannot be rewritten as client initialization"
    );

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"features":{"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("live readiness capability downgrade should succeed");
    assert!(!runtime.room_readiness.contains_key("room"));
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"features":{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#,
        )
        .expect("same-membership readiness re-upgrade should succeed");
    let restored = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(restored.membership_epoch, membership_epoch);
    assert_eq!(restored.user_intent, UserReadinessIntent::Ready);
}

#[test]
fn disabled_readiness_does_not_negotiate_or_attach_v2() {
    let mut runtime = ServerRuntime::default();
    runtime.set_readiness_enabled(false);
    let lines = runtime
        .handle_line_fanout("alice-client", &readiness_hello("alice", "room"))
        .expect("hello should still succeed");
    let hello = decode_directed_lines(&lines)
        .into_iter()
        .find_map(|(_, message)| match message {
            ProtocolMessage::Hello(hello) => Some(hello.hello),
            _ => None,
        })
        .expect("server hello should be present");
    assert_eq!(
        hello
            .features
            .as_ref()
            .and_then(|features| features.get(SOROTTE_READINESS_V2))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(!runtime.room_readiness.contains_key("room"));
    assert!(readiness_snapshot_for(&lines, "alice-client").is_none());

    let ignored_intent = ReadinessIntentRequest::new(
        "disabled-readiness-intent",
        1,
        1,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    );
    let ignored = ProtocolMessage::set(
        SetPayload::new()
            .with_readiness_v2(ReadinessSetExtension::new().with_intent(ignored_intent)),
    );
    assert!(
        runtime
            .handle_line_fanout(
                "alice-client",
                &encode_message_line(&ignored).expect("disabled readiness command should encode"),
            )
            .expect("disabled readiness command should be ignored")
            .is_empty()
    );
    assert!(!runtime.room_readiness.contains_key("room"));
}

#[test]
fn disabling_readiness_after_negotiation_does_not_leave_a_stale_v2_gate() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    assert!(runtime.room_readiness.contains_key("room"));

    runtime.set_readiness_enabled(false);
    start_barrier(&mut runtime, "alice-client");
    send_barrier_ready(&mut runtime, "alice-client", 1, true);

    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.phase, PlaybackBarrierPhase::Committed);
    assert_eq!(
        barrier.readiness_revision, None,
        "a disabled readiness coordinator must not bind a stale revision"
    );
}

#[test]
fn malformed_indirect_player_intent_is_rejected_without_consuming_nonce_or_mutating_intent() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    let initial_room_revision = runtime.room_readiness["room"].revision;

    for (operation_id, desired, action) in [
        (
            "pause-cannot-mean-ready",
            UserReadinessIntent::Ready,
            PlayerReadinessAction::Pause,
        ),
        (
            "play-cannot-mean-not-ready",
            UserReadinessIntent::NotReady,
            PlayerReadinessAction::Play,
        ),
    ] {
        let rejected = send_intent_with_source(
            &mut runtime,
            "alice-client",
            operation_id,
            1,
            epoch,
            desired,
            UserReadinessMutationSource::IndirectPlayer {
                action,
                surface: PlayerInteractionSurface::NativePlayerControl,
            },
        );
        assert_eq!(
            readiness_result_for(&rejected, "alice-client").map(|result| result.status),
            Some(ReadinessRequestResultStatus::RejectedInvalid)
        );
    }
    let actor = &runtime.room_readiness["room"].participants["alice"];
    assert_eq!(actor.highest_request_nonce, 0);
    assert!(actor.accepted_operations.is_empty());
    assert_eq!(actor.record.user_intent, UserReadinessIntent::NotReady);
    assert_eq!(actor.record.user_intent_revision, 0);
    assert_eq!(
        runtime.room_readiness["room"].revision,
        initial_room_revision
    );

    let accepted = send_intent(
        &mut runtime,
        "alice-client",
        "valid-after-malformed",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&accepted, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Accepted),
        "invalid source/desired pairs must not consume the request nonce"
    );
}

#[test]
fn v2_pause_ownership_requires_an_accepted_indirect_pause_in_either_message_order() {
    let pause_source = UserReadinessMutationSource::IndirectPlayer {
        action: PlayerReadinessAction::Pause,
        surface: PlayerInteractionSurface::NativePlayerControl,
    };

    let mut intent_first = ServerRuntime::default();
    intent_first.set_time_now_override_seconds(Some(100.0));
    intent_first
        .handle_line(
            "alice-client",
            &readiness_hello("alice", "intent-first-room"),
        )
        .expect("readiness hello should succeed");
    let epoch = intent_first.room_readiness["intent-first-room"].participants["alice"]
        .record
        .membership_epoch;
    intent_first
        .room_playback_state_mut("intent-first-room")
        .paused = false;
    acknowledge_forced_state(&mut intent_first, "alice-client");
    send_intent_with_source(
        &mut intent_first,
        "alice-client",
        "pause-before-telemetry",
        1,
        epoch,
        UserReadinessIntent::NotReady,
        pause_source.clone(),
    );
    assert_eq!(
        intent_first.room_readiness["intent-first-room"].pause_owner,
        RoomPauseOwner::None,
        "accepted intent must not claim a pause before transport is actually paused"
    );
    assert!(
        intent_first
            .pending_user_transport_by_client
            .contains_key("alice-client")
    );
    send_playstate(&mut intent_first, "alice-client", 10.0, true);
    assert_eq!(
        intent_first.room_readiness["intent-first-room"].pause_owner,
        RoomPauseOwner::User {
            actor: "alice".to_owned(),
        }
    );
    assert!(
        !intent_first
            .pending_user_transport_by_client
            .contains_key("alice-client")
    );

    let mut telemetry_first = ServerRuntime::default();
    telemetry_first
        .handle_line(
            "alice-client",
            &readiness_hello("alice", "telemetry-first-room"),
        )
        .expect("readiness hello should succeed");
    let epoch = telemetry_first.room_readiness["telemetry-first-room"].participants["alice"]
        .record
        .membership_epoch;
    telemetry_first
        .room_playback_state_mut("telemetry-first-room")
        .paused = false;
    acknowledge_forced_state(&mut telemetry_first, "alice-client");
    send_playstate(&mut telemetry_first, "alice-client", 20.0, true);
    assert!(
        !telemetry_first
            .room_playback_state("telemetry-first-room")
            .paused,
        "unclassified V2 telemetry must not mutate canonical transport"
    );
    assert_eq!(
        telemetry_first.room_readiness["telemetry-first-room"].pause_owner,
        RoomPauseOwner::None,
        "bare V2 pause telemetry is system/unknown-origin observation, not user intent"
    );
    acknowledge_forced_state(&mut telemetry_first, "alice-client");
    send_playstate(&mut telemetry_first, "alice-client", 20.0, true);
    assert_eq!(
        telemetry_first.room_readiness["telemetry-first-room"].pause_owner,
        RoomPauseOwner::None,
        "a duplicate paused heartbeat must not fabricate User ownership"
    );
    send_intent_with_source(
        &mut telemetry_first,
        "alice-client",
        "pause-after-telemetry",
        1,
        epoch,
        UserReadinessIntent::NotReady,
        pause_source,
    );
    assert_eq!(
        telemetry_first.room_readiness["telemetry-first-room"].pause_owner,
        RoomPauseOwner::User {
            actor: "alice".to_owned(),
        },
        "the later accepted indirect Pause attributes the already-observed pause"
    );
    assert!(
        telemetry_first
            .room_playback_state("telemetry-first-room")
            .paused
    );
}

#[test]
fn v2_play_transport_requires_matching_indirect_play_in_either_message_order() {
    let play_source = UserReadinessMutationSource::IndirectPlayer {
        action: PlayerReadinessAction::Play,
        surface: PlayerInteractionSurface::NativePlayerControl,
    };

    let mut telemetry_first = ServerRuntime::default();
    telemetry_first
        .handle_line(
            "alice-client",
            &readiness_hello("alice", "telemetry-first-play-room"),
        )
        .expect("readiness hello should succeed");
    let epoch = telemetry_first.room_readiness["telemetry-first-play-room"].participants["alice"]
        .record
        .membership_epoch;
    telemetry_first.claim_user_pause_ownership("telemetry-first-play-room", "alice");
    acknowledge_forced_state(&mut telemetry_first, "alice-client");
    send_playstate(&mut telemetry_first, "alice-client", 10.0, false);
    assert!(
        telemetry_first
            .room_playback_state("telemetry-first-play-room")
            .paused,
        "bare V2 unpause telemetry cannot release a User-owned pause"
    );
    assert_eq!(
        telemetry_first.room_readiness["telemetry-first-play-room"].pause_owner,
        RoomPauseOwner::User {
            actor: "alice".to_owned(),
        }
    );
    send_intent_with_source(
        &mut telemetry_first,
        "alice-client",
        "play-after-telemetry",
        1,
        epoch,
        UserReadinessIntent::Ready,
        play_source.clone(),
    );
    assert!(
        !telemetry_first
            .room_playback_state("telemetry-first-play-room")
            .paused
    );
    assert_eq!(
        telemetry_first.room_readiness["telemetry-first-play-room"].pause_owner,
        RoomPauseOwner::None
    );

    let mut intent_first = ServerRuntime::default();
    intent_first
        .handle_line(
            "alice-client",
            &readiness_hello("alice", "intent-first-play-room"),
        )
        .expect("readiness hello should succeed");
    let epoch = intent_first.room_readiness["intent-first-play-room"].participants["alice"]
        .record
        .membership_epoch;
    intent_first.claim_user_pause_ownership("intent-first-play-room", "alice");
    acknowledge_forced_state(&mut intent_first, "alice-client");
    send_intent_with_source(
        &mut intent_first,
        "alice-client",
        "play-before-telemetry",
        1,
        epoch,
        UserReadinessIntent::Ready,
        play_source,
    );
    assert!(
        intent_first
            .room_playback_state("intent-first-play-room")
            .paused
    );
    assert!(
        intent_first
            .pending_user_transport_by_client
            .contains_key("alice-client")
    );
    send_playstate(&mut intent_first, "alice-client", 20.0, false);
    assert!(
        !intent_first
            .room_playback_state("intent-first-play-room")
            .paused
    );
    assert_eq!(
        intent_first.room_readiness["intent-first-play-room"].pause_owner,
        RoomPauseOwner::None
    );
}

#[test]
fn newer_room_pause_authority_fences_stale_pending_play_but_allows_a_fresh_play() {
    let mut runtime = ServerRuntime::default();
    // Keep every event on the same wall-clock timestamp. Ordering must come
    // from canonical authority revisions rather than timestamp precision.
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &readiness_hello(username, "room"))
            .expect("readiness hello should succeed");
        acknowledge_forced_state(&mut runtime, client_id);
    }
    let alice_epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    let bob_epoch = runtime.room_readiness["room"].participants["bob"]
        .record
        .membership_epoch;
    let indirect_play = UserReadinessMutationSource::IndirectPlayer {
        action: PlayerReadinessAction::Play,
        surface: PlayerInteractionSurface::NativePlayerControl,
    };

    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "alice-play-before-bob-pause",
        1,
        alice_epoch,
        UserReadinessIntent::Ready,
        indirect_play.clone(),
    );
    let stale_authority_revision =
        runtime.pending_user_transport_by_client["alice-client"].transport_authority_revision;
    assert_eq!(
        runtime.pending_user_transport_by_client["alice-client"].evidence,
        PendingUserTransportEvidence::AcceptedIndirectAction
    );

    send_intent_with_source(
        &mut runtime,
        "bob-client",
        "bob-authoritative-pause",
        1,
        bob_epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::User {
            actor: "bob".to_owned(),
        }
    );
    assert!(runtime.room_readiness["room"].transport_authority_revision > stale_authority_revision);
    assert!(
        !runtime
            .pending_user_transport_by_client
            .contains_key("alice-client"),
        "Bob's newer pause authority must retire Alice's older pending Play"
    );

    acknowledge_forced_state(&mut runtime, "alice-client");
    let correction = send_playstate(&mut runtime, "alice-client", 10.0, false);
    assert!(
        decode_directed_lines(&correction)
            .into_iter()
            .any(|(client_id, message)| {
                client_id == "alice-client"
                    && matches!(
                        message,
                        ProtocolMessage::State(state)
                            if state.state.playstate.as_ref().is_some_and(|playstate| {
                                playstate.paused == Some(true)
                            })
                    )
            }),
        "Alice's stale physical Playing evidence must be corrected to Bob's pause"
    );
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::User {
            actor: "bob".to_owned(),
        }
    );
    assert_eq!(
        runtime.pending_user_transport_by_client["alice-client"].evidence,
        PendingUserTransportEvidence::UnclassifiedObservation,
        "the stale edge can only become fresh evidence under Bob's authority boundary"
    );

    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "alice-fresh-play-after-bob-pause",
        2,
        alice_epoch,
        UserReadinessIntent::Ready,
        indirect_play,
    );
    assert!(!runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_playback_state("room").set_by.as_deref(),
        Some("alice")
    );
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::None
    );
    assert!(runtime.pending_user_transport_by_client.is_empty());
}

#[test]
fn periodic_room_state_refresh_preserves_transport_pairing_in_both_message_orders() {
    for (telemetry_first, label, expected_evidence) in [
        (
            false,
            "intent-first",
            PendingUserTransportEvidence::AcceptedIndirectAction,
        ),
        (
            true,
            "telemetry-first",
            PendingUserTransportEvidence::UnclassifiedObservation,
        ),
    ] {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime
            .handle_line("alice-client", &readiness_hello("alice", label))
            .expect("readiness hello should succeed");
        acknowledge_forced_state(&mut runtime, "alice-client");
        runtime
            .handle_line(
                "alice-client",
                r#"{"Set":{"file":{"name":"movie.mkv","duration":120.0}}}"#,
            )
            .expect("the periodic refresh needs a current media participant");
        runtime.record_client_playback_state_sample("alice-client", Some(10.0), 100.0);
        let epoch = runtime.room_readiness[label].participants["alice"]
            .record
            .membership_epoch;
        let indirect_play = UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Play,
            surface: PlayerInteractionSurface::NativePlayerControl,
        };

        if telemetry_first {
            send_playstate(&mut runtime, "alice-client", 10.0, false);
            acknowledge_forced_state(&mut runtime, "alice-client");
        } else {
            send_intent_with_source(
                &mut runtime,
                "alice-client",
                &format!("{label}-play"),
                1,
                epoch,
                UserReadinessIntent::Ready,
                indirect_play.clone(),
            );
        }
        let pending_before_refresh =
            runtime.pending_user_transport_by_client["alice-client"].clone();
        assert_eq!(pending_before_refresh.evidence, expected_evidence);

        let refresh_at = 100.0 + SERVER_STATE_INTERVAL_SECONDS + 0.1;
        assert!(
            refresh_at < 100.0 + READINESS_USER_TRANSPORT_GRACE_SECONDS,
            "the regression must refresh inside the transport pairing grace window"
        );
        runtime.set_time_now_override_seconds(Some(refresh_at));
        runtime
            .collect_periodic_tick_for_client_at("alice-client", refresh_at, refresh_at)
            .expect("periodic room state refresh should succeed");
        assert_eq!(
            runtime.room_playback_state(label).updated_at_seconds,
            refresh_at,
            "the test must exercise a real periodic canonical-state refresh"
        );
        assert_eq!(
            runtime.pending_user_transport_by_client["alice-client"], pending_before_refresh,
            "routine position/setBy maintenance is not newer transport authority"
        );

        if telemetry_first {
            send_intent_with_source(
                &mut runtime,
                "alice-client",
                &format!("{label}-play"),
                1,
                epoch,
                UserReadinessIntent::Ready,
                indirect_play,
            );
        } else {
            send_playstate(&mut runtime, "alice-client", 10.1, false);
        }
        assert!(
            !runtime.room_playback_state(label).paused,
            "{label} pairing should still apply the user Play after periodic refresh"
        );
        assert_eq!(
            runtime.room_readiness[label].pause_owner,
            RoomPauseOwner::None
        );
        assert!(runtime.pending_user_transport_by_client.is_empty());
    }
}

#[test]
fn ask_controller_play_and_pause_retire_awaiting_decision_in_either_message_order() {
    for (desired_paused, telemetry_first, label) in [
        (false, false, "intent-first-play"),
        (false, true, "telemetry-first-play"),
        (true, false, "intent-first-pause"),
        (true, true, "telemetry-first-pause"),
    ] {
        let (mut runtime, epoch) = awaiting_controller_decision_runtime(label, !desired_paused);
        let (desired, action) = if desired_paused {
            (UserReadinessIntent::NotReady, PlayerReadinessAction::Pause)
        } else {
            (UserReadinessIntent::Ready, PlayerReadinessAction::Play)
        };
        let source = UserReadinessMutationSource::IndirectPlayer {
            action,
            surface: PlayerInteractionSurface::NativePlayerControl,
        };

        if telemetry_first {
            send_playstate(&mut runtime, "alice-client", 8.0, desired_paused);
            assert!(
                runtime
                    .pending_user_transport_by_client
                    .contains_key("alice-client"),
                "{label} must stage the unclassified telemetry edge"
            );
            acknowledge_forced_state(&mut runtime, "alice-client");
            send_intent_with_source(
                &mut runtime,
                "alice-client",
                &format!("{label}-intent"),
                1,
                epoch,
                desired,
                source,
            );
        } else {
            send_intent_with_source(
                &mut runtime,
                "alice-client",
                &format!("{label}-intent"),
                1,
                epoch,
                desired,
                source,
            );
            assert!(
                runtime
                    .pending_user_transport_by_client
                    .contains_key("alice-client"),
                "{label} must await the matching telemetry edge"
            );
            send_playstate(&mut runtime, "alice-client", 8.0, desired_paused);
        }

        assert_eq!(
            runtime.room_playback_barriers[label].phase,
            PlaybackBarrierPhase::Degraded,
            "{label} must retire AwaitingDecision"
        );
        assert!(runtime.room_playback_barriers[label].commit.is_none());
        assert_eq!(
            runtime.room_playback_state(label).paused,
            desired_paused,
            "{label} must apply the chosen canonical transport"
        );
        assert_eq!(
            runtime.room_readiness[label].pause_owner,
            if desired_paused {
                RoomPauseOwner::User {
                    actor: "alice".to_owned(),
                }
            } else {
                RoomPauseOwner::None
            }
        );
        assert_eq!(
            runtime.room_readiness[label].start_gate_phase,
            RoomStartGatePhase::Degraded {
                media_generation: 1,
                reason: StartGateDegradedReason::TimedOut,
            },
            "manual resolution must not erase the terminal timeout history"
        );
        assert!(runtime.pending_user_transport_by_client.is_empty());
    }

    for (desired_paused, label) in [
        (false, "already-playing-play"),
        (true, "already-paused-pause"),
    ] {
        let (mut runtime, epoch) = awaiting_controller_decision_runtime(label, desired_paused);
        send_intent_with_source(
            &mut runtime,
            "alice-client",
            &format!("{label}-intent"),
            1,
            epoch,
            if desired_paused {
                UserReadinessIntent::NotReady
            } else {
                UserReadinessIntent::Ready
            },
            UserReadinessMutationSource::IndirectPlayer {
                action: if desired_paused {
                    PlayerReadinessAction::Pause
                } else {
                    PlayerReadinessAction::Play
                },
                surface: PlayerInteractionSurface::NativePlayerControl,
            },
        );
        assert_eq!(
            runtime.room_playback_barriers[label].phase,
            PlaybackBarrierPhase::Degraded,
            "an authorized already-at-target decision must still retire AwaitingDecision"
        );
        assert_eq!(runtime.room_playback_state(label).paused, desired_paused);
    }

    let controlled_room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &readiness_hello(username, &controlled_room))
            .expect("readiness hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("Alice should authenticate as controller");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"readiness:controlled-ask","targetPosition":0.0,"policy":"allEligible","timeoutMs":1000,"timeoutAction":"askController"}}}}"#,
        )
        .expect("controlled AskController barrier should start");
    runtime
        .collect_dispatch_at(101.0)
        .expect("controlled AskController barrier should time out");
    let bob_epoch = runtime.room_readiness[&controlled_room].participants["bob"]
        .record
        .membership_epoch;
    send_intent_with_source(
        &mut runtime,
        "bob-client",
        "unauthorized-pause-decision",
        1,
        bob_epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    assert_eq!(
        runtime.room_playback_barriers[&controlled_room].phase,
        PlaybackBarrierPhase::AwaitingDecision,
        "a non-controller readiness mutation cannot supply the transport decision"
    );
}

#[test]
fn accepted_user_pause_retires_committed_barrier_before_every_started_ack() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let mut epochs = BTreeMap::new();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &readiness_hello(username, "room"))
            .expect("readiness hello should succeed");
        let epoch = runtime.room_readiness["room"].participants[username]
            .record
            .membership_epoch;
        epochs.insert(username, epoch);
        send_intent(
            &mut runtime,
            client_id,
            &format!("{username}-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
    }
    start_barrier(&mut runtime, "alice-client");
    for client_id in ["alice-client", "bob-client"] {
        send_technical(
            &mut runtime,
            client_id,
            unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
        );
        send_barrier_ready(&mut runtime, client_id, 1, true);
    }
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    let state_revision = runtime.room_playback_barriers["room"]
        .state_revision
        .expect("commit should have an authoritative revision");
    send_started(&mut runtime, "alice-client", 1, state_revision, 12.1);
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["alice-client"]
            .status
            .phase,
        PlaybackBarrierParticipantPhase::Started
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["bob-client"]
            .status
            .phase,
        PlaybackBarrierParticipantPhase::Ready
    );

    acknowledge_forced_state(&mut runtime, "alice-client");
    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "alice-pause-after-commit",
        2,
        epochs["alice"],
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    send_playstate(&mut runtime, "alice-client", 12.2, true);

    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.phase, PlaybackBarrierPhase::Degraded);
    assert_eq!(
        barrier.commit.as_ref().map(|commit| commit.state_revision),
        Some(state_revision),
        "the retired commit remains available as lifecycle history"
    );
    assert!(barrier.started_deadline.is_none());
    assert_eq!(
        barrier.participants["alice-client"].status.phase,
        PlaybackBarrierParticipantPhase::Started,
        "Alice's Started acknowledgement remains historical evidence"
    );
    assert_eq!(
        barrier.participants["bob-client"].status.phase,
        PlaybackBarrierParticipantPhase::Degraded
    );
    assert_eq!(
        barrier.participants["bob-client"].status.degraded_reason,
        Some(PlaybackBarrierDegradedReason::UserInterrupted)
    );
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::User {
            actor: "alice".to_owned(),
        }
    );
}

#[test]
fn pending_user_transport_expires_and_is_cleared_by_a_new_generation() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    runtime.room_playback_state_mut("room").paused = false;
    acknowledge_forced_state(&mut runtime, "alice-client");
    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "pause-that-times-out",
        1,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    runtime
        .set_time_now_override_seconds(Some(100.0 + READINESS_USER_TRANSPORT_GRACE_SECONDS + 1.0));
    send_playstate(&mut runtime, "alice-client", 30.0, true);
    assert!(!runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::None,
        "a later unrelated pause cannot consume an expired user marker"
    );

    acknowledge_forced_state(&mut runtime, "alice-client");
    runtime.set_time_now_override_seconds(Some(
        100.0 + (2.0 * READINESS_USER_TRANSPORT_GRACE_SECONDS) + 2.0,
    ));
    runtime.room_playback_state_mut("room").paused = false;
    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "pause-before-generation",
        2,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    assert!(
        runtime
            .pending_user_transport_by_client
            .contains_key("alice-client")
    );
    let stale = send_intent_with_source(
        &mut runtime,
        "alice-client",
        "stale-pause-operation",
        1,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    assert_eq!(
        readiness_result_for(&stale, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedStaleNonce)
    );
    assert!(runtime.pending_user_transport_by_client.is_empty());

    send_intent_with_source(
        &mut runtime,
        "alice-client",
        "pause-before-generation-rearmed",
        3,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    assert!(
        runtime
            .pending_user_transport_by_client
            .contains_key("alice-client")
    );
    start_barrier(&mut runtime, "alice-client");
    assert!(runtime.pending_user_transport_by_client.is_empty());
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::ReadinessStartGate {
            media_generation: 1,
        }
    );
}

#[test]
fn readiness_intents_are_revisioned_idempotent_and_nonce_scoped() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;

    let accepted = send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready-1",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&accepted, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Accepted)
    );
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert!(alice.room_ready);
    assert!(matches!(
        alice
            .last_user_mutation
            .as_ref()
            .map(|mutation| &mutation.source),
        Some(ReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        })
    ));
    assert_eq!(
        alice
            .last_user_mutation
            .as_ref()
            .and_then(|mutation| mutation.actor.as_deref()),
        Some("alice"),
        "the server derives the authenticated actor"
    );

    let duplicate = send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready-1",
        2,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&duplicate, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Duplicate)
    );

    let stale = send_intent(
        &mut runtime,
        "alice-client",
        "alice-stale-1",
        1,
        epoch,
        UserReadinessIntent::NotReady,
    );
    assert_eq!(
        readiness_result_for(&stale, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedStaleNonce)
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready,
        "a stale operation cannot overwrite newer intent"
    );
}

#[test]
fn self_intent_cas_uses_participant_revision_not_unrelated_room_revision() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("Alice should join");
    runtime
        .handle_line("bob-client", &readiness_hello("bob", "room"))
        .expect("Bob should join");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    let room_revision_before = runtime.room_readiness["room"].revision;
    runtime.set_readiness_pause_owner("room", RoomPauseOwner::Recovery, false);
    assert!(runtime.room_readiness["room"].revision > room_revision_before);

    let accepted_request = ReadinessIntentRequest::new(
        "alice-ready-cas",
        1,
        epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    )
    .with_expected_user_intent_revision(0);
    let accepted_message = ProtocolMessage::set(
        SetPayload::new()
            .with_readiness_v2(ReadinessSetExtension::new().with_intent(accepted_request)),
    );
    let accepted = runtime
        .handle_line_fanout(
            "alice-client",
            &encode_message_line(&accepted_message).expect("CAS request should encode"),
        )
        .expect("unrelated room revision must not conflict");
    let accepted_result =
        readiness_result_for(&accepted, "alice-client").expect("result should be returned");
    assert_eq!(
        accepted_result.status,
        ReadinessRequestResultStatus::Accepted
    );
    assert_eq!(accepted_result.user_intent_revision, Some(1));

    runtime.set_readiness_pause_owner("room", RoomPauseOwner::None, false);
    let stale_request = ReadinessIntentRequest::new(
        "alice-stale-cas",
        2,
        epoch,
        UserReadinessIntent::NotReady,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    )
    .with_expected_user_intent_revision(0);
    let stale_message = ProtocolMessage::set(
        SetPayload::new()
            .with_readiness_v2(ReadinessSetExtension::new().with_intent(stale_request)),
    );
    let rejected = runtime
        .handle_line_fanout(
            "alice-client",
            &encode_message_line(&stale_message).expect("stale CAS request should encode"),
        )
        .expect("stale CAS request should receive a result");
    let rejected_result =
        readiness_result_for(&rejected, "alice-client").expect("result should be returned");
    assert_eq!(
        rejected_result.status,
        ReadinessRequestResultStatus::RejectedRevisionConflict
    );
    assert_eq!(rejected_result.user_intent_revision, Some(1));
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );
}

#[test]
fn same_room_reconnect_preserves_intent_but_room_switch_allocates_fresh_membership() {
    let mut runtime = ServerRuntime::default();
    let initial = runtime
        .handle_line_fanout("alice-client", &readiness_hello("alice", "room-a"))
        .expect("initial hello should succeed");
    let reconnect_token = reconnect_token_for(&initial, "alice-client");
    let original_epoch = runtime.room_readiness["room-a"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready-before-reconnect",
        1,
        original_epoch,
        UserReadinessIntent::Ready,
    );

    let rehello = runtime
        .handle_line_fanout(
            "alice-client",
            &readiness_hello_with_token("alice", "room-a", &reconnect_token),
        )
        .expect("same-room rehello should reconcile membership");
    assert_eq!(
        reconnect_token_for(&rehello, "alice-client"),
        reconnect_token,
        "the stable membership token survives a lost Hello response and retry"
    );
    let restored = &runtime.room_readiness["room-a"].participants["alice"].record;
    assert_eq!(restored.membership_epoch, original_epoch);
    assert_eq!(restored.user_intent, UserReadinessIntent::Ready);

    runtime
        .handle_line_fanout("alice-client", r#"{"Set":{"room":{"name":"room-b"}}}"#)
        .expect("room switch should succeed");
    let fresh = &runtime.room_readiness["room-b"].participants["alice"].record;
    assert_ne!(fresh.membership_epoch, original_epoch);
    assert_eq!(fresh.user_intent, UserReadinessIntent::NotReady);
    assert!(!fresh.room_ready);

    let stale_old_membership = send_intent(
        &mut runtime,
        "alice-client",
        "delayed-old-room-operation",
        2,
        original_epoch,
        UserReadinessIntent::NotReady,
    );
    assert_eq!(
        readiness_result_for(&stale_old_membership, "alice-client").map(|result| result.status),
        Some(ReadinessRequestResultStatus::RejectedStaleMembership)
    );
}

#[test]
fn reconnect_resets_connection_nonce_without_losing_operation_idempotency() {
    let mut runtime = ServerRuntime::default();
    let initial = runtime
        .handle_line_fanout("alice-old", &readiness_hello("alice", "room"))
        .expect("initial hello should succeed");
    let reconnect_token = reconnect_token_for(&initial, "alice-old");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    let initial = send_intent(
        &mut runtime,
        "alice-old",
        "operation-before-reconnect",
        100,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&initial, "alice-old").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Accepted)
    );
    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("disconnect should retain reconnect state");
    runtime
        .handle_line(
            "alice-new",
            &readiness_hello_with_token("alice", "room", &reconnect_token),
        )
        .expect("reconnect should restore membership");
    let restored = &runtime.room_readiness["room"].participants["alice"];
    assert_eq!(restored.record.membership_epoch, epoch);
    assert_eq!(restored.highest_request_nonce, 0);

    let duplicate = send_intent(
        &mut runtime,
        "alice-new",
        "operation-before-reconnect",
        100,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&duplicate, "alice-new").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Duplicate)
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"].highest_request_nonce, 0,
        "a replayed old operation must not poison the fresh connection nonce space"
    );

    let new_operation = send_intent(
        &mut runtime,
        "alice-new",
        "new-operation-after-reconnect",
        1,
        epoch,
        UserReadinessIntent::NotReady,
    );
    assert_eq!(
        readiness_result_for(&new_operation, "alice-new").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Accepted)
    );
    let superseded_replay = send_intent(
        &mut runtime,
        "alice-new",
        "operation-before-reconnect",
        100,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        readiness_result_for(&superseded_replay, "alice-new").map(|result| result.status),
        Some(ReadinessRequestResultStatus::Superseded)
    );
    let participant = &runtime.room_readiness["room"].participants["alice"];
    assert_eq!(participant.highest_request_nonce, 1);
    assert_eq!(
        participant.record.user_intent,
        UserReadinessIntent::NotReady,
        "replaying an acknowledged old operation remains side-effect free"
    );
}

#[test]
fn same_username_without_reconnect_token_gets_fresh_intent_and_epoch() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &readiness_hello("alice", "room"))
        .expect("initial hello should succeed");
    let original_epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-old",
        "original-ready",
        1,
        original_epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-old");
    send_technical(
        &mut runtime,
        "alice-old",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::Playable {
            media_generation: 1
        }
    ));
    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("disconnect should retain only token-protected continuity state");

    runtime
        .handle_line("alice-unproven", &readiness_hello("alice", "room"))
        .expect("same display name may join without proving continuity");
    let replacement = &runtime.room_readiness["room"].participants["alice"].record;
    assert_ne!(replacement.membership_epoch, original_epoch);
    assert_eq!(replacement.user_intent, UserReadinessIntent::NotReady);
    assert_eq!(replacement.user_intent_revision, 0);
    assert_eq!(replacement.last_technical_report_sequence, 0);
    assert!(
        !matches!(
            replacement.technical_state,
            TechnicalPlayability::Playable { .. }
        ),
        "an unproven replacement transport must not inherit the old transport's Playable state"
    );
    assert!(!replacement.start_eligible);
}

#[test]
fn restored_start_eligible_membership_rechecks_active_gate_on_reconnect() {
    let mut runtime = ServerRuntime::default();
    runtime.set_mixed_readiness_policy(MixedReadinessPolicy::ExcludeLegacy);
    let mut reconnect_tokens = BTreeMap::new();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        let hello = runtime
            .handle_line_fanout(client_id, &readiness_hello(username, "room"))
            .expect("readiness hello should succeed");
        reconnect_tokens.insert(username, reconnect_token_for(&hello, client_id));
        let epoch = runtime.room_readiness["room"].participants[username]
            .record
            .membership_epoch;
        send_intent(
            &mut runtime,
            client_id,
            &format!("{username}-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
    }
    runtime
        .handle_line("legacy-observer", &legacy_hello("legacy", "room"))
        .expect("legacy observer should keep the active room alive");
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );

    runtime
        .handle_transport_disconnect_fanout("alice-client")
        .expect("eligible participant disconnect should succeed");
    runtime
        .handle_transport_disconnect_fanout("bob-client")
        .expect("blocking participant disconnect should succeed");
    assert!(!runtime.room_readiness.contains_key("room"));
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );

    let reconnect = runtime
        .handle_line_fanout(
            "alice-reconnected",
            &readiness_hello_with_token("alice", "room", &reconnect_tokens["alice"]),
        )
        .expect("same-room reconnect should restore membership");
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert!(matches!(
        alice.technical_state,
        TechnicalPlayability::Preparing {
            media_generation: 1
        }
    ));
    assert!(!alice.start_eligible);
    assert!(
        !decode_directed_lines(&reconnect)
            .iter()
            .any(|(recipient, message)| {
                recipient == "alice-reconnected"
                    && barrier_extension(message)
                        .is_some_and(|extension| extension.commit.is_some())
            }),
        "a replacement transport must not inherit transient playability or barrier readiness"
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["alice-reconnected"]
            .status
            .phase,
        PlaybackBarrierParticipantPhase::Pending
    );

    send_technical(
        &mut runtime,
        "alice-reconnected",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "fresh generic playability is still not barrier-target evidence"
    );
    send_barrier_ready(&mut runtime, "alice-reconnected", 1, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
}

#[test]
fn room_switch_orders_local_room_echo_before_new_readiness_snapshot() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room-a"))
        .expect("initial hello should succeed");

    let lines = runtime
        .handle_line_fanout("alice-client", r#"{"Set":{"room":{"name":"room-b"}}}"#)
        .expect("room switch should succeed");
    let messages = decode_directed_lines(&lines);
    let local_messages: Vec<_> = messages
        .iter()
        .filter(|(recipient, _)| recipient == "alice-client")
        .map(|(_, message)| message)
        .collect();
    let room_echo_index = local_messages
        .iter()
        .position(|message| match message {
            ProtocolMessage::Set(payload) => payload
                .set
                .user
                .as_ref()
                .and_then(|users| users.get("alice"))
                .and_then(|user| user.room.as_ref())
                .is_some_and(|room| room.name == "room-b"),
            _ => false,
        })
        .expect("moving client should receive its new-room echo");
    let new_snapshot_index = local_messages
        .iter()
        .position(|message| {
            readiness_extension(message)
                .and_then(|extension| extension.snapshot)
                .is_some_and(|snapshot| {
                    snapshot.participants.get("alice").is_some_and(|alice| {
                        alice.membership_epoch
                            == runtime.room_readiness["room-b"].participants["alice"]
                                .record
                                .membership_epoch
                    })
                })
        })
        .expect("moving client should receive its new readiness membership");

    assert!(
        room_echo_index < new_snapshot_index,
        "the room boundary must precede the unscoped new-room readiness snapshot"
    );
}

#[test]
fn automatic_start_waits_for_intent_and_playability_then_binds_readiness_revision() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &readiness_hello(username, "room"))
            .expect("readiness hello should succeed");
        let epoch = runtime.room_readiness["room"].participants[username]
            .record
            .membership_epoch;
        send_intent(
            &mut runtime,
            client_id,
            &format!("{username}-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
    }

    start_barrier(&mut runtime, "alice-client");
    for participant in runtime.room_readiness["room"].participants.values() {
        assert!(
            participant.record.room_ready,
            "intent survives media change"
        );
        assert!(!participant.record.start_eligible);
        assert!(matches!(
            participant.record.technical_state,
            TechnicalPlayability::Preparing {
                media_generation: 1
            }
        ));
    }
    assert!(runtime.room_playback_state("room").paused);

    let bob_ready = send_technical(
        &mut runtime,
        "bob-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(
        decode_directed_lines(&bob_ready)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| { extension.commit.is_none() })),
        "one technically ready participant must not start the room"
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );
    send_barrier_ready(&mut runtime, "bob-client", 1, true);

    let alice_playable = send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(
        decode_directed_lines(&alice_playable)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| extension.commit.is_none())),
        "generic Playable must not manufacture Alice's target-specific barrier evidence"
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );
    let committed = send_barrier_ready(&mut runtime, "alice-client", 1, true);
    let commits: Vec<_> = decode_directed_lines(&committed)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message)?.commit)
        .collect();
    assert_eq!(
        commits.len(),
        2,
        "one canonical commit is fanned out once per peer"
    );
    let readiness_revision = runtime.room_playback_barriers["room"]
        .readiness_revision
        .expect("V2 commit must retain its evaluated readiness revision");
    assert!(
        commits
            .iter()
            .all(|commit| commit.readiness_revision == Some(readiness_revision))
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert!(!runtime.room_playback_state("room").paused);
    assert!(matches!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Committed {
            media_generation: 1,
            readiness_revision: bound_revision,
            playback_revision: 1,
        } if bound_revision == readiness_revision
    ));

    let duplicate = send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(
        decode_directed_lines(&duplicate)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| { extension.commit.is_none() })),
        "duplicate technical telemetry must not create another commit"
    );
    assert_eq!(runtime.next_playback_barrier_revision, 1);
}

#[test]
fn loaded_media_without_applied_target_cannot_commit_an_otherwise_eligible_participant() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-target-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .start_eligible,
        "user intent and technical playability should leave target application as the only blocker"
    );

    let unapplied = send_barrier_observation(
        &mut runtime,
        "alice-client",
        MediaReadyPayload::new(1, true, false).with_seekable(true),
    );
    assert!(
        decode_directed_lines(&unapplied)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| extension.commit.is_none())),
        "loaded media without buffer-ready target evidence must not commit"
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["alice-client"]
            .status
            .phase,
        PlaybackBarrierParticipantPhase::Pending
    );
    assert_eq!(runtime.next_playback_barrier_revision, 0);
    assert!(runtime.room_playback_state("room").paused);

    let applied = send_barrier_observation(
        &mut runtime,
        "alice-client",
        MediaReadyPayload::new(1, true, true).with_seekable(true),
    );
    let commits: Vec<_> = decode_directed_lines(&applied)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message)?.commit)
        .collect();
    assert_eq!(commits.len(), 1);
    assert_eq!(runtime.next_playback_barrier_revision, 1);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn explicit_wait_all_settings_wait_for_every_ready_and_playable_participant() {
    let stored_wait_all = StoredClientSettingsV1 {
        streaming_start_policy: Some("wait-all".to_owned()),
        ..StoredClientSettingsV1::default()
    };
    let configured_wait_all = ClientConfig::try_from_stored(&stored_wait_all)
        .expect("explicit wait-all stored settings should resolve");
    assert_eq!(
        configured_wait_all
            .playback
            .streaming
            .start_synchronization
            .policy,
        StartSynchronizationPolicy::WaitForAllEligible
    );
    assert_eq!(
        StartSynchronizationConfig::default().policy,
        StartSynchronizationPolicy::Immediate
    );
    assert_eq!(
        PlaybackBarrierStartConfig::default().policy,
        None,
        "the core coordinator default must preserve immediate legacy starts"
    );

    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let alice_join = runtime
        .handle_line_fanout("alice-client", &readiness_hello("alice", "room"))
        .expect("Alice readiness hello should succeed");
    runtime
        .handle_line("bob-client", &readiness_hello("bob", "room"))
        .expect("Bob readiness hello should succeed");

    let mut client_session = ClientSession::default();
    for line in alice_join
        .iter()
        .filter(|line| line.client_id == "alice-client")
    {
        client_session
            .apply_message_json(&line.line)
            .expect("Alice application should accept the server join state");
    }
    let mut application = ClientApplication::new(client_session, DisconnectedPlayer);
    application.dispatch(ClientCommand::update_settings(
        ClientApplicationSettings::new(configured_wait_all).with_active_room("room"),
    ));

    let alice_epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-default-ready",
        1,
        alice_epoch,
        UserReadinessIntent::Ready,
    );

    application.prepare_playback_media(
        LogicalMediaId::new("readiness:default-policy-media")
            .expect("test media identity should be valid"),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    let pending = application
        .pending_protocol_line()
        .expect("explicit wait-all application prepare should encode")
        .expect("explicit wait-all application should queue a barrier prepare");
    let prepare_line = pending.line().to_owned();
    let ProtocolMessage::Set(set) =
        decode_message_line(&prepare_line).expect("application prepare should decode")
    else {
        panic!("application playback coordination should use a Set envelope");
    };
    let prepare = set
        .set
        .playback_barrier_v1()
        .expect("application barrier extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("application should emit a barrier prepare");
    assert_eq!(
        prepare.policy,
        PlaybackBarrierPolicy::AllEligible,
        "explicit wait-all stored settings must reach the wire as allEligible"
    );
    application
        .acknowledge_protocol_line(pending.lease())
        .expect("transport write should release the serialized prepare");
    runtime
        .handle_line_fanout("alice-client", &prepare_line)
        .expect("server should start the barrier from the application-emitted request");
    assert!(runtime.room_playback_state("room").paused);
    assert!(
        !runtime.room_readiness["room"].participants["bob"]
            .record
            .room_ready
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["bob"]
            .record
            .technical_state,
        TechnicalPlayability::Preparing {
            media_generation: 1
        }
    ));

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    send_barrier_ready(&mut runtime, "bob-client", 1, true);
    assert!(
        runtime.room_playback_state("room").paused,
        "a wait-all V2 room must stay paused while Bob is Not Ready"
    );
    assert_eq!(runtime.next_playback_barrier_revision, 0);

    let bob_epoch = runtime.room_readiness["room"].participants["bob"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "bob-client",
        "bob-default-ready",
        1,
        bob_epoch,
        UserReadinessIntent::Ready,
    );
    assert!(
        runtime.room_playback_state("room").paused,
        "Ready intent alone must not bypass Bob's Preparing technical state"
    );
    assert_eq!(runtime.next_playback_barrier_revision, 0);

    let committed = send_technical(
        &mut runtime,
        "bob-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    let commits: Vec<_> = decode_directed_lines(&committed)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message)?.commit)
        .collect();
    assert_eq!(
        commits.len(),
        2,
        "one canonical commit should be fanned out once to each participant"
    );
    assert_eq!(runtime.next_playback_barrier_revision, 1);
    assert!(!runtime.room_playback_state("room").paused);

    let duplicate = send_technical(
        &mut runtime,
        "bob-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert!(
        decode_directed_lines(&duplicate)
            .iter()
            .all(|(_, message)| barrier_extension(message)
                .is_none_or(|extension| extension.commit.is_none()))
    );
    assert_eq!(
        runtime.next_playback_barrier_revision, 1,
        "duplicate playability must not create a second commit"
    );
}

#[test]
fn new_generation_does_not_steal_user_pause_without_explicit_ready_rearm() {
    let mut runtime = ServerRuntime::default();
    runtime.set_mixed_readiness_policy(MixedReadinessPolicy::ExcludeLegacy);
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    runtime
        .handle_line("legacy-client", &legacy_hello("legacy", "room"))
        .expect("legacy peer should join");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    runtime.claim_user_pause_ownership("room", "legacy");

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","logicalMediaId":"readiness:replacement","targetPosition":0.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
        )
        .expect("same initiator should replace the active generation");
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::User {
            actor: "legacy".to_owned(),
        }
    );
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Degraded {
            media_generation: 2,
            reason: StartGateDegradedReason::UserPaused,
        }
    );

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(2, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 2, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "technical readiness alone cannot release another user's pause"
    );
    assert!(runtime.room_playback_state("room").paused);

    send_intent(
        &mut runtime,
        "alice-client",
        "alice-explicit-rearm",
        2,
        epoch,
        UserReadinessIntent::Ready,
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed,
        "an explicit Ready action may re-arm and release the readiness gate"
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn new_generation_retires_superseded_automatic_pause_owners() {
    for automatic_owner in [
        RoomPauseOwner::Recovery,
        RoomPauseOwner::RoomBufferingPolicy {
            media_generation: 1,
            state_revision: Some(7),
        },
    ] {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line("alice-client", &readiness_hello("alice", "room"))
            .expect("readiness hello should succeed");
        start_barrier(&mut runtime, "alice-client");
        runtime.set_readiness_pause_owner("room", automatic_owner, false);

        runtime
            .handle_line_fanout(
                "alice-client",
                r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","logicalMediaId":"readiness:replacement","targetPosition":0.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
            )
            .expect("same initiator should replace the automatic episode");

        assert_eq!(
            runtime.room_readiness["room"].pause_owner,
            RoomPauseOwner::ReadinessStartGate {
                media_generation: 2,
            },
            "a fresh generation must retire any superseded automatic owner"
        );
    }
}

#[test]
fn eof_pause_is_system_owned_and_hands_off_to_the_next_readiness_generation() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    runtime.room_playlist_state_mut("room").index = Some(0);
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert!(!runtime.room_playback_state("room").paused);
    let forced_state_counter = runtime.server_ignoring_counter("alice-client");
    runtime.acknowledge_server_ignoring_counter("alice-client", forced_state_counter);

    runtime
        .handle_line_fanout("alice-client", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("next playlist index should be accepted before coalesced pause telemetry");
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::EndOfPlaylist
    );
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":60.0,"paused":true,"doSeek":false}}}"#,
        )
        .expect("EOF pause sample should apply");
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::EndOfPlaylist,
        "automatic EOF telemetry must not be relabelled as a user pause"
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","logicalMediaId":"readiness:next-item","targetPosition":0.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
        )
        .expect("next playlist item should open a fresh generation");
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::ReadinessStartGate {
            media_generation: 2,
        }
    );
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(2, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 2, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn telemetry_first_final_eof_preserves_system_pause_ownership_without_playlist_change() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert!(!runtime.room_playback_state("room").paused);
    acknowledge_forced_state(&mut runtime, "alice-client");

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Preparing)
            .with_reason(TechnicalBlockCause::EndOfFile),
    );
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::None,
        "telemetry may precede the player's final paused sample"
    );
    send_playstate(&mut runtime, "alice-client", 60.0, true);
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::EndOfPlaylist,
        "Preparing + EndOfFile must retain provenance until the final pause arrives"
    );
    let alice = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(alice.user_intent, UserReadinessIntent::Ready);
    assert!(alice.room_ready);
    assert!(!alice.start_eligible);
}

#[test]
fn excluded_legacy_controller_eof_pause_preserves_system_ownership_and_v2_commit() {
    let mut runtime = ServerRuntime::default();
    runtime.set_mixed_readiness_policy(MixedReadinessPolicy::ExcludeLegacy);
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    runtime.room_playlist_state_mut("room").index = Some(0);
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert!(!runtime.room_playback_state("room").paused);

    runtime
        .handle_line("legacy-client", &legacy_hello("legacy", "room"))
        .expect("legacy controller should join");
    acknowledge_forced_state(&mut runtime, "legacy-client");
    runtime
        .handle_line_fanout("legacy-client", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("legacy controller should advance the playlist");
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::EndOfPlaylist
    );
    send_playstate(&mut runtime, "legacy-client", 60.0, true);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::EndOfPlaylist,
        "excluded legacy automatic telemetry must not fabricate User ownership"
    );
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","logicalMediaId":"readiness:legacy-next-item","targetPosition":0.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
        )
        .expect("V2 controller should open the next generation");
    assert_eq!(
        runtime.room_playback_barriers["room"].excluded_legacy_clients,
        BTreeSet::from(["legacy".to_owned()])
    );
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::ReadinessStartGate {
            media_generation: 2,
        }
    );
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(2, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 2, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn recovery_ownership_tracks_an_actual_paused_episode_and_releases_on_unpause() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert!(!runtime.room_playback_state("room").paused);
    acknowledge_forced_state(&mut runtime, "alice-client");

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::TemporarilyBlocked)
            .with_reason(TechnicalBlockCause::Recovery)
            .with_recovery(RecoveryStage::Retrying),
    );
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::None,
        "a recovery report while canonical transport is playing must not claim a pause"
    );

    send_playstate(&mut runtime, "alice-client", 40.0, true);
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::Recovery,
        "the later system pause is attributed to the active recovery episode"
    );
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::Recovery,
        "Playable telemetry alone must fail closed while the recovery-owned pause remains"
    );

    acknowledge_forced_state(&mut runtime, "alice-client");
    send_playstate(&mut runtime, "alice-client", 41.0, false);
    assert!(!runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_readiness["room"].pause_owner,
        RoomPauseOwner::None
    );
}

#[test]
fn disconnecting_blocking_required_participant_rechecks_and_commits_readiness_gate() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &readiness_hello(username, "room"))
            .expect("readiness hello should succeed");
        let epoch = runtime.room_readiness["room"].participants[username]
            .record
            .membership_epoch;
        send_intent(
            &mut runtime,
            client_id,
            &format!("{username}-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
    }

    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "Bob's required preparing membership should still block the gate"
    );

    let disconnected = runtime
        .handle_transport_disconnect_fanout("bob-client")
        .expect("disconnect should re-evaluate the readiness cohort");
    assert!(
        decode_directed_lines(&disconnected)
            .iter()
            .any(|(recipient, message)| {
                recipient == "alice-client"
                    && barrier_extension(message)
                        .is_some_and(|extension| extension.commit.is_some())
            }),
        "the remaining eligible participant should receive the automatic-start commit"
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn terminal_prepare_timeout_actions_publish_a_timed_out_readiness_gate() {
    for (wire_action, expected_barrier_phase) in [
        ("remainPaused", PlaybackBarrierPhase::Degraded),
        ("askController", PlaybackBarrierPhase::AwaitingDecision),
    ] {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime
            .handle_line("alice-client", &readiness_hello("alice", "room"))
            .expect("readiness hello should succeed");
        runtime
            .handle_line_fanout(
                "alice-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"readiness:timeout","targetPosition":0.0,"policy":"allEligible","timeoutMs":1000,"timeoutAction":"{wire_action}"}}}}}}}}"#
                ),
            )
            .expect("playback barrier should start");
        assert!(matches!(
            runtime.room_readiness["room"].start_gate_phase,
            RoomStartGatePhase::WaitingForIntent {
                media_generation: 1
            }
        ));

        let timed_out = runtime
            .collect_dispatch_at(101.0)
            .expect("terminal timeout should be collected");
        assert_eq!(
            runtime.room_playback_barriers["room"].phase,
            expected_barrier_phase
        );
        let expected_gate_phase = RoomStartGatePhase::Degraded {
            media_generation: 1,
            reason: StartGateDegradedReason::TimedOut,
        };
        assert_eq!(
            runtime.room_readiness["room"].start_gate_phase,
            expected_gate_phase
        );
        assert_eq!(
            readiness_snapshot_for(&timed_out.outbound_lines, "alice-client")
                .map(|snapshot| snapshot.start_gate_phase),
            Some(expected_gate_phase.clone()),
            "{wire_action} must replace the stale waiting snapshot with the terminal gate state"
        );
        assert!(runtime.room_playback_state("room").paused);

        let participant = &runtime.room_readiness["room"].participants["alice"].record;
        let epoch = participant.membership_epoch;
        let next_sequence = participant.last_technical_report_sequence.saturating_add(1);
        let wrongly_revisioned = TechnicalReadinessReport::new(
            1,
            epoch,
            next_sequence,
            TechnicalPlayabilityPhase::Playable,
        )
        .with_authoritative_playback_revision(99);
        assert!(
            send_technical_exact(&mut runtime, "alice-client", wrongly_revisioned).is_empty(),
            "{wire_action} has no commit, so a revision-bound report must be rejected"
        );
        assert_eq!(
            runtime.room_readiness["room"].participants["alice"]
                .record
                .last_technical_report_sequence,
            next_sequence.saturating_sub(1)
        );

        send_technical(
            &mut runtime,
            "alice-client",
            unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
        );
        assert_eq!(
            runtime.room_readiness["room"].start_gate_phase, expected_gate_phase,
            "{wire_action} must retain TimedOut after a technical refresh"
        );
        send_intent(
            &mut runtime,
            "alice-client",
            &format!("{wire_action}-late-ready"),
            1,
            epoch,
            UserReadinessIntent::Ready,
        );
        assert_eq!(
            runtime.room_readiness["room"].start_gate_phase, expected_gate_phase,
            "{wire_action} must retain TimedOut after a readiness refresh"
        );
    }
}

#[test]
fn continue_timeout_cannot_bypass_or_stall_on_user_pause_ownership() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    runtime.claim_user_pause_ownership("room", "legacy");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    assert!(runtime.playback_barrier_policy_satisfied("room"));
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing,
        "the pre-timeout commit must respect user pause ownership"
    );

    runtime
        .collect_dispatch_at(105.0)
        .expect("continue timeout should terminate the blocked gate");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Degraded
    );
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Degraded {
            media_generation: 1,
            reason: StartGateDegradedReason::TimedOut,
        }
    );
    assert!(runtime.room_playback_state("room").paused);
}

#[test]
fn stale_playback_revision_cannot_be_bridged_into_media_ready() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    runtime
        .room_playback_barriers
        .get_mut("room")
        .expect("barrier should exist")
        .state_revision = Some(9);

    let stale = send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable)
            .with_authoritative_playback_revision(8),
    );
    assert!(
        stale.is_empty(),
        "stale report should be ignored atomically"
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::Preparing {
            media_generation: 1
        }
    ));
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["alice-client"]
            .status
            .phase,
        sorotte_protocol::PlaybackBarrierParticipantPhase::Pending,
        "the stale readiness extension must not be downgraded into an unrevisioned MediaReady"
    );
}

#[test]
fn technical_reports_reject_stale_epoch_duplicate_sequence_and_out_of_order_recovery() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    start_barrier(&mut runtime, "alice-client");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;

    let terminal =
        TechnicalReadinessReport::new(1, epoch, 2, TechnicalPlayabilityPhase::TerminallyBlocked)
            .with_reason(TechnicalBlockCause::PlayerFailure)
            .with_observed_at(20.0);
    assert!(!send_technical_exact(&mut runtime, "alice-client", terminal).is_empty());
    assert_eq!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .last_technical_report_sequence,
        2
    );

    for stale in [
        TechnicalReadinessReport::new(1, epoch, 1, TechnicalPlayabilityPhase::Playable)
            .with_observed_at(10.0),
        TechnicalReadinessReport::new(1, epoch, 2, TechnicalPlayabilityPhase::Playable)
            .with_observed_at(20.0),
        TechnicalReadinessReport::new(
            1,
            epoch.saturating_add(1),
            99,
            TechnicalPlayabilityPhase::Playable,
        )
        .with_observed_at(99.0),
        TechnicalReadinessReport::new(1, epoch, 3, TechnicalPlayabilityPhase::Playable)
            .with_observed_at(19.0),
    ] {
        assert!(
            send_technical_exact(&mut runtime, "alice-client", stale).is_empty(),
            "stale ordering evidence must be ignored atomically"
        );
    }
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::TerminallyBlocked {
            cause: TechnicalBlockCause::PlayerFailure,
            ..
        }
    ));

    let recovered = TechnicalReadinessReport::new(1, epoch, 3, TechnicalPlayabilityPhase::Playable)
        .with_observed_at(21.0);
    assert!(!send_technical_exact(&mut runtime, "alice-client", recovered).is_empty());
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::Playable { .. }
    ));
}

#[test]
fn post_commit_technical_block_requires_authoritative_server_revision() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    let state_revision = runtime.room_playback_barriers["room"]
        .state_revision
        .expect("commit should allocate an authoritative state revision");
    let next_sequence = runtime.room_readiness["room"].participants["alice"]
        .record
        .last_technical_report_sequence
        .saturating_add(1);

    let missing_revision = TechnicalReadinessReport::new(
        1,
        epoch,
        next_sequence,
        TechnicalPlayabilityPhase::TemporarilyBlocked,
    )
    .with_reason(TechnicalBlockCause::Rebuffering);
    assert!(
        send_technical_exact(&mut runtime, "alice-client", missing_revision).is_empty(),
        "an omitted revision cannot bypass an active authoritative fence"
    );
    let correctly_bound = TechnicalReadinessReport::new(
        1,
        epoch,
        next_sequence,
        TechnicalPlayabilityPhase::TemporarilyBlocked,
    )
    .with_authoritative_playback_revision(state_revision)
    .with_reason(TechnicalBlockCause::Rebuffering);
    assert!(!send_technical_exact(&mut runtime, "alice-client", correctly_bound).is_empty());
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::TemporarilyBlocked {
            cause: TechnicalBlockCause::Rebuffering,
            ..
        }
    ));
}

#[test]
fn post_commit_degraded_barrier_retains_authoritative_revision_for_technical_reports() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    send_barrier_ready(&mut runtime, "alice-client", 1, true);
    let (state_revision, started_deadline) = {
        let barrier = &runtime.room_playback_barriers["room"];
        (
            barrier
                .state_revision
                .expect("commit should allocate a state revision"),
            barrier
                .started_deadline
                .expect("commit should arm the Started deadline"),
        )
    };

    runtime
        .collect_dispatch_at(started_deadline)
        .expect("Started timeout should be collected");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Degraded
    );
    assert!(runtime.room_playback_barriers["room"].commit.is_some());

    let next_sequence = runtime.room_readiness["room"].participants["alice"]
        .record
        .last_technical_report_sequence
        .saturating_add(1);
    assert!(
        send_technical_exact(
            &mut runtime,
            "alice-client",
            TechnicalReadinessReport::new(
                1,
                epoch,
                next_sequence,
                TechnicalPlayabilityPhase::TemporarilyBlocked,
            )
            .with_reason(TechnicalBlockCause::Rebuffering),
        )
        .is_empty(),
        "retained post-commit authority must reject an unrevisioned report"
    );

    assert!(
        !send_technical(
            &mut runtime,
            "alice-client",
            unscoped_technical_report(1, TechnicalPlayabilityPhase::TemporarilyBlocked)
                .with_reason(TechnicalBlockCause::Rebuffering),
        )
        .is_empty()
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::TemporarilyBlocked {
            cause: TechnicalBlockCause::Rebuffering,
            ..
        }
    ));
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Inactive,
        "a refreshed post-commit degraded barrier no longer owns the active start gate"
    );

    assert!(
        !send_technical(
            &mut runtime,
            "alice-client",
            unscoped_technical_report(1, TechnicalPlayabilityPhase::TerminallyBlocked)
                .with_reason(TechnicalBlockCause::PlayerFailure),
        )
        .is_empty()
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::TerminallyBlocked {
            cause: TechnicalBlockCause::PlayerFailure,
            ..
        }
    ));

    assert!(
        !send_technical(
            &mut runtime,
            "alice-client",
            unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
        )
        .is_empty()
    );
    assert!(matches!(
        runtime.room_readiness["room"].participants["alice"]
            .record
            .technical_state,
        TechnicalPlayability::Playable { .. }
    ));
    assert_eq!(
        runtime.room_playback_barriers["room"].state_revision,
        Some(state_revision)
    );
}

#[test]
fn temporary_v2_technical_block_is_not_erased_by_media_ready_bridge() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");

    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::TemporarilyBlocked)
            .with_reason(TechnicalBlockCause::Rebuffering)
            .with_recovery(RecoveryStage::Retrying),
    );
    let record = &runtime.room_readiness["room"].participants["alice"].record;
    assert!(matches!(
        record.technical_state,
        TechnicalPlayability::TemporarilyBlocked {
            media_generation: 1,
            cause: TechnicalBlockCause::Rebuffering,
            recovery: RecoveryStage::Retrying,
        }
    ));
    assert_eq!(record.user_intent, UserReadinessIntent::Ready);
    assert!(
        record.room_ready,
        "temporary blocks preserve room readiness"
    );
    assert!(!record.start_eligible);
    let barrier_status =
        &runtime.room_playback_barriers["room"].participants["alice-client"].status;
    assert_eq!(
        barrier_status.phase,
        PlaybackBarrierParticipantPhase::Pending
    );
    assert_eq!(
        barrier_status
            .readiness
            .as_ref()
            .map(|ready| ready.is_ready()),
        None
    );
}

#[test]
fn terminal_failure_preserves_intent_and_recovery_does_not_override_later_not_ready() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &readiness_hello("alice", "room"))
        .expect("readiness hello should succeed");
    let epoch = runtime.room_readiness["room"].participants["alice"]
        .record
        .membership_epoch;
    send_intent(
        &mut runtime,
        "alice-client",
        "alice-ready",
        1,
        epoch,
        UserReadinessIntent::Ready,
    );
    start_barrier(&mut runtime, "alice-client");

    let failed = send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::TerminallyBlocked)
            .with_reason(TechnicalBlockCause::RecoveryExhausted),
    );
    let record = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(record.user_intent, UserReadinessIntent::Ready);
    assert!(matches!(
        record.technical_state,
        TechnicalPlayability::TerminallyBlocked {
            media_generation: 1,
            cause: TechnicalBlockCause::RecoveryExhausted,
        }
    ));
    assert!(record.terminal_technical_block.is_some());
    assert!(!record.room_ready);
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Degraded {
            media_generation: 1,
            reason: StartGateDegradedReason::TechnicalFailure,
        },
        "retained Ready intent must expose the terminal technical blocker separately"
    );
    assert!(has_ready_update(
        &decode_directed_lines(&failed),
        "alice-client",
        "alice",
        false
    ));

    send_intent(
        &mut runtime,
        "alice-client",
        "alice-not-ready-during-failure",
        2,
        epoch,
        UserReadinessIntent::NotReady,
    );
    send_technical(
        &mut runtime,
        "alice-client",
        unscoped_technical_report(1, TechnicalPlayabilityPhase::Playable),
    );
    let recovered = &runtime.room_readiness["room"].participants["alice"].record;
    assert_eq!(recovered.user_intent, UserReadinessIntent::NotReady);
    assert!(recovered.terminal_technical_block.is_none());
    assert!(!recovered.room_ready);
    assert!(!recovered.start_eligible);
    assert!(runtime.room_playback_state("room").paused);
}
