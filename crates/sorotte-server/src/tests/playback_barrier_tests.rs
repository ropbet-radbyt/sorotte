use super::*;
use crate::ServerCompatibilityFallback;
use sorotte_protocol::{
    MediaLoadIntent, PlaybackBarrierDegradedReason, PlaybackBarrierParticipantPhase,
    PlaybackBarrierPhase, PlaybackBarrierSetExtension, PlaybackBarrierStatusPayload,
    RoomBufferingPhase, RoomBufferingPolicy, RoomBufferingStatusPayload,
    SOROTTE_PLAYBACK_BARRIER_V1,
};

const CAPABILITY: &str = r#""sorottePlaybackBarrierV1":true"#;

fn hello(username: &str, room: &str, capability: bool) -> String {
    let features = if capability {
        format!(r#","features":{{{CAPABILITY}}}"#)
    } else {
        String::new()
    };
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5"{features}}}}}"#
    )
}

fn barrier_extension(message: &ProtocolMessage) -> Option<PlaybackBarrierSetExtension> {
    let ProtocolMessage::Set(set) = message else {
        return None;
    };
    set.set.playback_barrier_v1().ok().flatten()
}

fn messages(lines: &[DirectedOutboundLine]) -> Vec<(String, ProtocolMessage)> {
    decode_directed_lines(lines)
}

fn buffering_status(lines: &[DirectedOutboundLine]) -> Option<RoomBufferingStatusPayload> {
    messages(lines)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.buffering_status)
}

fn playback_barrier_status(lines: &[DirectedOutboundLine]) -> Option<PlaybackBarrierStatusPayload> {
    messages(lines)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
}

fn recipient_has_pause(lines: &[DirectedOutboundLine], recipient: &str, paused: bool) -> bool {
    messages(lines).into_iter().any(|(client_id, message)| {
        client_id == recipient
            && matches!(
                message,
                ProtocolMessage::State(state)
                    if state.state.playstate.as_ref().and_then(|playstate| playstate.paused)
                        == Some(paused)
            )
    })
}

fn authenticate_policy_controller(runtime: &mut ServerRuntime, client_id: &str) {
    runtime
        .handle_line_fanout(
            client_id,
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("policy controller authentication should succeed");
}

#[test]
fn hello_advertises_playback_barrier_and_prepare_is_hidden_from_legacy_clients() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    let alice_hello = runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("alice hello should succeed");
    runtime
        .handle_line("bob-client", &hello("bob", "room", true))
        .expect("bob hello should succeed");
    runtime
        .handle_line("legacy-client", &hello("legacy", "room", false))
        .expect("legacy hello should succeed");

    let server_hello = alice_hello
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find_map(|message| match message {
            ProtocolMessage::Hello(hello) => Some(hello.hello),
            _ => None,
        })
        .expect("server should respond with Hello");
    assert_eq!(
        server_hello
            .features
            .as_ref()
            .and_then(|features| features.get(SOROTTE_PLAYBACK_BARRIER_V1))
            .and_then(Value::as_bool),
        Some(true)
    );

    let lines = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"youtube:video","targetPosition":12.0,"policy":"allEligible","timeoutMs":5000}}}}"#,
        )
        .expect("authorized prepare should succeed");
    let decoded = messages(&lines);

    for recipient in ["alice-client", "bob-client"] {
        let extension = decoded
            .iter()
            .find_map(|(client_id, message)| {
                (client_id == recipient)
                    .then(|| barrier_extension(message))
                    .flatten()
            })
            .expect("capable participant should receive prepare extension");
        let prepare = extension.prepare.expect("prepare should be present");
        assert_eq!(prepare.media_generation, 1, "the server assigns generation");
        assert_eq!(prepare.request_nonce, 1, "the request nonce is echoed");
        assert_eq!(prepare.deadline, Some(105.0));
        let status = extension.status.expect("status should accompany prepare");
        assert_eq!(
            status.excluded_legacy_clients,
            BTreeSet::from(["legacy".to_owned()])
        );
    }
    assert!(
        decoded.iter().all(|(client_id, message)| {
            client_id != "legacy-client" || barrier_extension(message).is_none()
        }),
        "legacy clients must never receive the Sorotte extension"
    );
    assert!(decoded.iter().any(|(client_id, message)| {
        client_id == "legacy-client"
            && matches!(
                message,
                ProtocolMessage::State(state)
                    if state.state.playstate.as_ref().and_then(|playstate| playstate.paused)
                        == Some(true)
            )
    }));
}

#[test]
fn authenticated_readiness_commits_once_and_started_acks_complete_status() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":4,"loadIntent":"newPlayback","logicalMediaId":"plex:item","targetPosition":30.0,"policy":"allEligible","timeoutMs":20000}}}}"#,
        )
        .expect("prepare should succeed");

    let bob_ready = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true,"username":"alice"}}}}"#,
        )
        .expect("bob readiness should succeed");
    let bob_status = messages(&bob_ready)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("readiness should publish status");
    assert_eq!(
        bob_status.participants["bob"].phase,
        PlaybackBarrierParticipantPhase::Ready
    );
    assert_eq!(
        bob_status.participants["alice"].phase,
        PlaybackBarrierParticipantPhase::Pending,
        "wire username must not let bob acknowledge alice"
    );

    let commit_lines = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":30.0,"paused":true,"doSeek":false},"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("alice readiness should commit the barrier");
    let commit_messages = messages(&commit_lines);
    let commits: Vec<_> = commit_messages
        .iter()
        .filter_map(|(_, message)| barrier_extension(message)?.commit)
        .collect();
    assert_eq!(commits.len(), 2, "each capable participant gets one commit");
    assert!(commits.iter().all(|commit| {
        commit.media_generation == 1 && commit.state_revision == 1 && commit.anchor_position == 30.0
    }));
    assert!(
        commit_messages.iter().all(|(_, message)| {
            !matches!(
                message,
                ProtocolMessage::State(state)
                    if state.state.playstate.as_ref().and_then(|playstate| playstate.paused)
                        == Some(true)
            )
        }),
        "stale paused sample bundled with MediaReady must not undo commit"
    );
    assert!(!runtime.room_playback_state("room").paused);

    let alice_started = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":30.1}}}}"#,
        )
        .expect("alice StartedAck should succeed");
    let alice_status = messages(&alice_started)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("StartedAck should publish status");
    assert_eq!(alice_status.phase, PlaybackBarrierPhase::Committed);

    let bob_started = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":30.2}}}}"#,
        )
        .expect("bob StartedAck should succeed");
    let final_status = messages(&bob_started)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("final StartedAck should publish status");
    assert_eq!(final_status.phase, PlaybackBarrierPhase::Complete);
    assert!(
        final_status
            .participants
            .values()
            .all(|participant| { participant.phase == PlaybackBarrierParticipantPhase::Started })
    );
}

#[test]
fn prepare_quorum_percent_is_normalized_after_capable_cohort_capture() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username, capable) in [
        ("alice-client", "alice", true),
        ("bob-client", "bob", true),
        ("charlie-client", "charlie", true),
        ("legacy-client", "legacy", false),
    ] {
        runtime
            .handle_line(client_id, &hello(username, "room", capable))
            .expect("hello should succeed");
    }
    let prepared = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":5,"loadIntent":"newPlayback","logicalMediaId":"youtube:item","targetPosition":0.0,"policy":"quorum","quorum":1,"quorumPercent":34}}}}"#,
        )
        .expect("percentage quorum prepare should succeed");
    let extension = messages(&prepared)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find(|extension| extension.prepare.is_some())
        .expect("capable cohort should receive normalized prepare");
    let prepare = extension.prepare.expect("prepare should be present");
    assert_eq!(prepare.quorum_percent, Some(34));
    assert_eq!(
        prepare.quorum,
        Some(2),
        "34 percent of three capable clients rounds up to two; legacy is excluded"
    );
    assert_eq!(extension.status.and_then(|status| status.quorum), Some(2));

    let first_ready = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("first readiness should succeed");
    assert!(
        messages(&first_ready)
            .iter()
            .filter_map(|(_, message)| barrier_extension(message))
            .all(|extension| extension.commit.is_none()),
        "absolute quorum=1 must not override the preferred percentage"
    );
    let second_ready = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("second readiness should commit");
    assert!(messages(&second_ready).iter().any(|(_, message)| {
        barrier_extension(message)
            .and_then(|extension| extension.commit)
            .is_some()
    }));
}

#[test]
fn controlled_room_rejects_prepare_from_non_controller() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("alice controller auth should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("controller should start room playback");

    let unauthorized = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"youtube:item","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("unauthorized prepare should be safely ignored");
    assert!(unauthorized.is_empty());
    assert!(!runtime.room_playback_barriers.contains_key(&room));

    let authorized = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"youtube:item","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("controller prepare should succeed");
    assert!(messages(&authorized).iter().any(|(_, message)| {
        barrier_extension(message)
            .and_then(|extension| extension.prepare)
            .is_some()
    }));
}

#[test]
fn peer_load_of_active_logical_media_cannot_replace_the_start_initiator() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"media-sha256:item","targetPosition":0.0,"policy":"allEligible"}}}}"#,
        )
        .expect("first prepare should succeed");

    let duplicate = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"replay","logicalMediaId":"media-sha256:item","targetPosition":0.0,"policy":"allEligible"}}}}"#,
        )
        .expect("peer duplicate should be safely ignored");
    assert!(duplicate.is_empty());
    let barrier = runtime
        .room_playback_barriers
        .get("room")
        .expect("original barrier should remain");
    assert_eq!(barrier.initiator_client_id, "alice-client");
    assert_eq!(barrier.prepare.media_generation, 1);
}

#[test]
fn server_generations_are_monotonic_and_terminal_requests_are_idempotent() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("hello should succeed");

    let client_claim = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":18446744073709551615,"requestNonce":40,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .expect("a client generation claim should be safely rejected");
    assert!(client_claim.is_empty());

    let request = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":41,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":2.0,"policy":"controller"}}}}"#;
    let first = runtime
        .handle_line_fanout("alice-client", request)
        .expect("first playback request should succeed");
    let first_extension = messages(&first)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find(|extension| extension.prepare.is_some())
        .expect("server should publish canonical prepare");
    assert_eq!(
        first_extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );

    let active_retry = runtime
        .handle_line_fanout("alice-client", request)
        .expect("active retry should replay canonical state");
    let active_retry_extension = messages(&active_retry)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .next()
        .expect("active retry should receive a snapshot");
    assert_eq!(
        active_retry_extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );
    assert_eq!(
        active_retry_extension.status.map(|status| status.phase),
        Some(PlaybackBarrierPhase::Preparing)
    );
    assert!(active_retry_extension.commit.is_none());

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("controller readiness should commit");
    let completed = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":2.1}}}}"#,
        )
        .expect("started ack should complete barrier");
    assert_eq!(
        playback_barrier_status(&completed).map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete)
    );

    let terminal_retry = runtime
        .handle_line_fanout("alice-client", request)
        .expect("terminal retry should replay, not allocate");
    let terminal_extension = messages(&terminal_retry)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .next()
        .expect("terminal retry should receive retained history");
    assert_eq!(
        terminal_extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );
    assert_eq!(
        terminal_extension
            .commit
            .as_ref()
            .map(|commit| commit.state_revision),
        Some(1)
    );
    assert_eq!(
        terminal_extension.status.map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete)
    );

    runtime
        .handle_line("bob-client", &hello("bob", "room", true))
        .expect("replacement controller should join");
    let refresh = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":99,"loadIntent":"transportRefresh","logicalMediaId":"item","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .expect("transport refresh should retain canonical generation");
    let refresh_extension = messages(&refresh)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .next()
        .expect("refresh should replay canonical lifecycle identity");
    assert_eq!(
        refresh_extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );
    assert_eq!(
        refresh_extension
            .commit
            .as_ref()
            .map(|commit| commit.state_revision),
        Some(1)
    );
    assert_eq!(
        refresh_extension
            .status
            .as_ref()
            .and_then(|status| status.state_revision),
        Some(1)
    );
    assert!(!runtime.room_playback_state("room").paused);

    let inferred_replay = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":100,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .expect("replacement controller should infer replay from terminal identity");
    let replay_prepare = messages(&inferred_replay)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.prepare)
        .expect("inferred replay should allocate a new canonical generation");
    assert_eq!(replay_prepare.media_generation, 2);
    assert_eq!(replay_prepare.load_intent, MediaLoadIntent::Replay);
    assert_eq!(replay_prepare.request_nonce, 100);

    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":2,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("replay controller should commit generation two");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":2,"stateRevision":2,"observedPosition":0.1}}}}"#,
        )
        .expect("alice should acknowledge replay start");
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":2,"stateRevision":2,"observedPosition":0.1}}}}"#,
        )
        .expect("bob should complete generation two");
    assert_eq!(
        runtime
            .room_playback_barriers
            .get("room")
            .map(|barrier| (barrier.prepare.media_generation, barrier.phase)),
        Some((2, PlaybackBarrierPhase::Complete))
    );

    let superseded_retry = runtime
        .handle_line_fanout("alice-client", request)
        .expect("superseded request retry should be safely suppressed");
    assert!(
        superseded_retry.is_empty(),
        "an old nonce must not become fresh playback intent after a newer terminal generation"
    );
    assert_eq!(
        runtime
            .room_playback_barriers
            .get("room")
            .map(|barrier| (barrier.prepare.media_generation, barrier.phase)),
        Some((2, PlaybackBarrierPhase::Complete))
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn barrier_disabled_new_media_supersedes_retained_terminal_generation() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(10.0));
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("hello should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"first","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("first prepare should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("first barrier should commit");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":0.1}}}}"#,
        )
        .expect("first barrier should complete");
    assert_eq!(
        runtime
            .room_playback_barriers
            .get("room")
            .map(|barrier| barrier.phase),
        Some(PlaybackBarrierPhase::Complete)
    );

    let policy_only_request = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","policy":"independent"}}}}"#;
    let next_media = runtime
        .handle_line_fanout("alice-client", policy_only_request)
        .expect("barrier-disabled media should still allocate server generation");
    let config = buffering_status(&next_media)
        .expect("server should publish canonical buffering generation")
        .config;
    assert_eq!(config.media_generation, 2);
    assert_eq!(config.request_nonce, 2);
    assert_eq!(config.load_intent, MediaLoadIntent::NewPlayback);
    assert!(
        !runtime.room_playback_barriers.contains_key("room"),
        "the terminal generation must not remain the current room-media lifecycle"
    );

    let retry = runtime
        .handle_line_fanout("alice-client", policy_only_request)
        .expect("policy-only retry should replay canonical config");
    assert_eq!(
        buffering_status(&retry).map(|status| status.config.media_generation),
        Some(2)
    );

    let third = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":3,"loadIntent":"newPlayback","logicalMediaId":"third","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("later start barrier should continue monotonic generation");
    assert_eq!(
        messages(&third)
            .into_iter()
            .filter_map(|(_, message)| barrier_extension(&message))
            .find_map(|extension| extension.prepare)
            .map(|prepare| prepare.media_generation),
        Some(3)
    );
}

#[test]
fn prepare_timeout_pause_actions_are_atomic_and_server_enforced() {
    for (wire_action, expected_phase) in [
        ("remainPaused", PlaybackBarrierPhase::Degraded),
        ("askController", PlaybackBarrierPhase::AwaitingDecision),
    ] {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime
            .handle_line("alice-client", &hello("alice", "room", true))
            .expect("hello should succeed");
        runtime
            .handle_line_fanout(
                "alice-client",
                r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false}}}"#,
            )
            .expect("room should start playing");
        runtime
            .handle_line_fanout(
                "alice-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":4.0,"policy":"controller","timeoutMs":1000,"timeoutAction":"{wire_action}"}}}}}}}}"#
                ),
            )
            .expect("prepare should succeed");
        assert!(runtime.room_playback_state("room").paused);

        let deadline = runtime
            .collect_dispatch_at(101.0)
            .expect("prepare timeout should be enforced by server");
        assert!(
            runtime.room_playback_state("room").paused,
            "{wire_action} must never transiently unpause the canonical room"
        );
        let decoded = messages(&deadline.outbound_lines);
        assert!(decoded.iter().all(|(_, message)| {
            barrier_extension(message).is_none_or(|extension| extension.commit.is_none())
        }));
        assert!(!recipient_has_pause(
            &deadline.outbound_lines,
            "alice-client",
            false
        ));
        let status = playback_barrier_status(&deadline.outbound_lines)
            .expect("timeout should publish terminal server status");
        assert_eq!(status.phase, expected_phase);
        assert!(status.participants.values().all(|participant| {
            participant.phase == PlaybackBarrierParticipantPhase::PrepareTimedOut
                && participant.degraded_reason
                    == Some(PlaybackBarrierDegradedReason::PrepareTimeout)
        }));
        if wire_action == "askController" {
            let forced_pause_counter = runtime.server_ignoring_counter("alice-client");
            runtime.acknowledge_server_ignoring_counter("alice-client", forced_pause_counter);
            runtime.set_time_now_override_seconds(Some(101.1));
            let decision = runtime
                .handle_line_fanout(
                    "alice-client",
                    r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false}}}"#,
                )
                .expect("ordinary controller play should resolve awaiting decision");
            assert!(!runtime.room_playback_state("room").paused);
            assert_eq!(
                playback_barrier_status(&decision).map(|status| status.phase),
                Some(PlaybackBarrierPhase::Degraded),
                "the manual decision must retire server-barrier authority"
            );
        }
    }
}

#[test]
fn started_ack_timeout_is_distinct_and_cannot_apply_prepare_timeout_policy() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(20.0));
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("hello should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":7,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":1.0,"policy":"controller","timeoutMs":1000,"timeoutAction":"remainPaused"}}}}"#,
        )
        .expect("prepare should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("readiness should commit before prepare timeout");
    assert!(!runtime.room_playback_state("room").paused);

    let started_timeout = runtime
        .collect_dispatch_at(30.0)
        .expect("started acknowledgement timeout should degrade");
    assert!(
        !runtime.room_playback_state("room").paused,
        "post-commit timeout must not reuse remain-paused prepare policy"
    );
    assert!(!recipient_has_pause(
        &started_timeout.outbound_lines,
        "alice-client",
        true
    ));
    let status = playback_barrier_status(&started_timeout.outbound_lines)
        .expect("started timeout should publish status");
    assert_eq!(status.phase, PlaybackBarrierPhase::Degraded);
    assert!(status.participants.values().all(|participant| {
        participant.phase == PlaybackBarrierParticipantPhase::StartedAckTimedOut
            && participant.degraded_reason == Some(PlaybackBarrierDegradedReason::StartedTimeout)
    }));
}

#[test]
fn preparation_and_started_deadlines_degrade_stalled_clients_without_holding_room() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    let prepare_lines = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":8,"loadIntent":"newPlayback","logicalMediaId":"stream:item","targetPosition":7.0,"policy":"allEligible","timeoutMs":999999}}}}"#,
        )
        .expect("prepare should succeed");
    let prepare = messages(&prepare_lines)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.prepare)
        .expect("prepare should be broadcast");
    assert_eq!(prepare.timeout_ms, Some(30_000));
    assert_eq!(prepare.deadline, Some(130.0));

    let timeout_dispatch = runtime
        .collect_dispatch_at(130.0)
        .expect("deadline should commit best effort");
    let timeout_messages = messages(&timeout_dispatch.outbound_lines);
    let timeout_status = timeout_messages
        .iter()
        .filter_map(|(_, message)| barrier_extension(message))
        .find_map(|extension| extension.status)
        .expect("timeout commit should publish status");
    assert_eq!(timeout_status.phase, PlaybackBarrierPhase::Committed);
    assert!(timeout_status.participants.values().all(|participant| {
        participant.phase == PlaybackBarrierParticipantPhase::PrepareTimedOut
            && participant.degraded_reason == Some(PlaybackBarrierDegradedReason::PrepareTimeout)
    }));
    assert!(!runtime.room_playback_state("room").paused);
    assert!(
        timeout_dispatch.outbound_lines.iter().any(|line| {
            line.delivery == ServerOutboundDelivery::Reliable
            && decode_message_line(&line.line).ok().is_some_and(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(state)
                        if state.state.playstate.as_ref().and_then(|playstate| playstate.do_seek)
                            == Some(true)
                )
            })
        }),
        "deadline commit's playstate transition must be reliable"
    );

    let started_timeout = runtime
        .collect_dispatch_at(140.0)
        .expect("StartedAck deadline should degrade stalled participants");
    let degraded = messages(&started_timeout.outbound_lines)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
        .expect("StartedAck timeout should publish degraded status");
    assert_eq!(degraded.phase, PlaybackBarrierPhase::Degraded);
}

#[test]
fn unadvertised_and_malformed_barrier_extensions_are_compatibly_ignored() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("legacy-client", &hello("legacy", "room", false))
        .expect("legacy hello should succeed");
    let unadvertised = runtime
        .handle_line_fanout(
            "legacy-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("valid but unadvertised extension should be ignored");
    assert!(unadvertised.is_empty());

    runtime
        .handle_line("capable-client", &hello("capable", "room", true))
        .expect("capable hello should succeed");
    let malformed = runtime
        .handle_line_fanout(
            "capable-client",
            r#"{"State":{"ping":{"clientRtt":0.1},"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":"invalid"}}}}"#,
        )
        .expect("malformed extension should not reject the compatible State envelope");
    assert!(malformed.is_empty());
    assert!(
        runtime
            .drain_compatibility_fallbacks()
            .iter()
            .any(|fallback| {
                matches!(
                    fallback,
                    ServerCompatibilityFallback::IgnoredInvalidPlaybackBarrier { context, .. }
                        if context == "State.sorottePlaybackBarrierV1"
                )
            })
    );
}

#[test]
fn disconnected_participant_is_degraded_and_does_not_hold_all_eligible_barrier() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":12,"loadIntent":"newPlayback","logicalMediaId":"item","targetPosition":3.0,"policy":"allEligible"}}}}"#,
        )
        .expect("prepare should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"bufferReady":true}}}}"#,
        )
        .expect("alice readiness should succeed");

    let disconnect = runtime
        .handle_transport_disconnect_fanout("bob-client")
        .expect("disconnect should update the barrier");
    let committed = messages(&disconnect)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find(|extension| extension.commit.is_some())
        .expect("remaining ready cohort should receive a commit");
    let status = committed.status.expect("commit should include status");
    assert_eq!(status.phase, PlaybackBarrierPhase::Committed);
    assert_eq!(
        status.participants["bob"].degraded_reason,
        Some(PlaybackBarrierDegradedReason::Disconnected)
    );
    assert!(!runtime.room_playback_state("room").paused);
}

#[test]
fn controlled_room_authorizes_policy_and_keeps_legacy_clients_wire_compatible() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username, capable) in [
        ("alice-client", "alice", true),
        ("bob-client", "bob", true),
        ("legacy-client", "legacy", false),
    ] {
        runtime
            .handle_line(client_id, &hello(username, &room, capable))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("alice controller auth should succeed");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("controller should start room playback");

    let unauthorized = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":1,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("unauthorized policy should be ignored");
    assert!(unauthorized.is_empty());
    assert!(!runtime.room_buffering_controls.contains_key(&room));

    let configured = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":1,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("authorized policy should succeed");
    let configured_messages = messages(&configured);
    assert!(configured_messages.iter().any(|(client_id, message)| {
        client_id == "alice-client"
            && barrier_extension(message)
                .and_then(|extension| extension.buffering_status)
                .is_some()
    }));
    assert!(configured_messages.iter().any(|(client_id, message)| {
        client_id == "bob-client"
            && barrier_extension(message)
                .and_then(|extension| extension.buffering_status)
                .is_some()
    }));
    assert!(configured_messages.iter().all(|(client_id, message)| {
        client_id != "legacy-client" || barrier_extension(message).is_none()
    }));

    let paused = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":1,"buffering":true,"username":"alice"}}}}"#,
        )
        .expect("session-bound report should succeed");
    assert!(runtime.room_playback_state(&room).paused);
    let status = buffering_status(&paused).expect("report should publish status");
    assert_eq!(status.phase, RoomBufferingPhase::Paused);
    assert_eq!(status.buffering_clients, BTreeSet::from(["bob".to_owned()]));
    assert_eq!(status.eligible_clients, 2, "legacy client is not eligible");
    assert!(
        recipient_has_pause(&paused, "legacy-client", true),
        "legacy participants still receive the ordinary canonical pause"
    );

    let legacy_spoof = runtime
        .handle_line_fanout(
            "legacy-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":1,"buffering":false}}}}"#,
        )
        .expect("unadvertised transport extension should be ignored");
    assert!(legacy_spoof.is_empty());
    assert!(runtime.room_playback_state(&room).paused);
}

#[test]
fn public_room_clients_cannot_enable_coordinated_buffering_pauses() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "public-room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("public room should start playing");

    for policy in ["pauseController", "pauseAnyEligible", "quorum"] {
        let attempted = runtime
            .handle_line_fanout(
                "alice-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"bufferingPolicy":{{"mediaGeneration":1,"policy":"{policy}","quorumPercent":50,"debounceMs":0}}}}}}}}"#
                ),
            )
            .expect("unauthorized public-room policy should be safely ignored");
        assert!(attempted.is_empty());
        assert!(!runtime.room_buffering_controls.contains_key("public-room"));
    }

    let independent = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":1,"policy":"independent"}}}}"#,
        )
        .expect("public clients may explicitly select harmless independent behavior");
    assert_eq!(
        buffering_status(&independent).map(|status| status.config.policy),
        Some(RoomBufferingPolicy::Independent)
    );
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":1,"buffering":true}}}}"#,
        )
        .expect("transport report under independent policy should be harmless");
    assert!(!runtime.room_playback_state("public-room").paused);
}

#[test]
fn buffering_pause_uses_debounce_and_resume_hysteresis() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(100.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":2,"policy":"pauseAnyEligible","debounceMs":1000,"resumeHysteresisMs":1500,"maxPauseMs":10000}}}}"#,
        )
        .expect("policy should configure");

    let began_buffering = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":2,"buffering":true}}}}"#,
        )
        .expect("buffering report should succeed");
    assert!(!runtime.room_playback_state(&room).paused);
    assert_eq!(
        buffering_status(&began_buffering).map(|status| status.phase),
        Some(RoomBufferingPhase::DebouncingPause)
    );

    runtime
        .collect_dispatch_at(100.99)
        .expect("pre-debounce maintenance should succeed");
    assert!(!runtime.room_playback_state(&room).paused);
    let paused = runtime
        .collect_dispatch_at(101.0)
        .expect("debounce deadline should pause");
    assert!(runtime.room_playback_state(&room).paused);
    assert!(recipient_has_pause(
        &paused.outbound_lines,
        "alice-client",
        true
    ));

    runtime.set_time_now_override_seconds(Some(101.1));
    let recovered = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":2,"buffering":false}}}}"#,
        )
        .expect("recovery report should succeed");
    assert_eq!(
        buffering_status(&recovered).map(|status| status.phase),
        Some(RoomBufferingPhase::DebouncingResume)
    );
    assert!(runtime.room_playback_state(&room).paused);
    runtime
        .collect_dispatch_at(102.59)
        .expect("pre-hysteresis maintenance should succeed");
    assert!(runtime.room_playback_state(&room).paused);
    let resumed = runtime
        .collect_dispatch_at(102.6)
        .expect("hysteresis deadline should resume");
    assert!(!runtime.room_playback_state(&room).paused);
    assert!(recipient_has_pause(
        &resumed.outbound_lines,
        "bob-client",
        false
    ));
}

#[test]
fn maximum_pause_fails_open_and_requires_a_recovered_interval_before_rearming() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(50.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    let configured = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":3,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":500,"maxPauseMs":10}}}}"#,
        )
        .expect("policy should configure");
    assert_eq!(
        buffering_status(&configured).and_then(|status| status.config.max_pause_ms),
        Some(1_000),
        "maximum pause is normalized to the safe minimum"
    );
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":3,"buffering":true}}}}"#,
        )
        .expect("buffering should pause immediately");
    assert!(runtime.room_playback_state(&room).paused);

    let failed_open = runtime
        .collect_dispatch_at(51.0)
        .expect("maximum pause deadline should fail open");
    assert!(!runtime.room_playback_state(&room).paused);
    assert_eq!(
        buffering_status(&failed_open.outbound_lines).map(|status| status.phase),
        Some(RoomBufferingPhase::FailOpen)
    );
    runtime
        .collect_dispatch_at(55.0)
        .expect("stalled report should remain fail-open");
    assert!(!runtime.room_playback_state(&room).paused);

    runtime.set_time_now_override_seconds(Some(55.0));
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":3,"buffering":false}}}}"#,
        )
        .expect("clear report should begin fail-open rearm");
    runtime
        .collect_dispatch_at(55.5)
        .expect("recovered interval should rearm policy");
    assert!(!runtime.room_playback_state(&room).paused);
    runtime.set_time_now_override_seconds(Some(55.6));
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":3,"buffering":true}}}}"#,
        )
        .expect("new buffering episode should be eligible after rearm");
    assert!(runtime.room_playback_state(&room).paused);
}

#[test]
fn buffering_reports_and_policy_updates_are_generation_and_revision_scoped() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(70.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":6,"stateRevision":7,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("policy should configure");

    for stale in [
        r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":5,"stateRevision":7,"buffering":true}}}}"#,
        r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":6,"stateRevision":6,"buffering":true}}}}"#,
        r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":6,"buffering":true}}}}"#,
    ] {
        let ignored = runtime
            .handle_line_fanout("bob-client", stale)
            .expect("stale report should be safely ignored");
        assert!(ignored.is_empty());
        assert!(!runtime.room_playback_state(&room).paused);
    }
    let stale_policy = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":5,"stateRevision":8,"policy":"pauseController"}}}}"#,
        )
        .expect("older policy should be safely ignored");
    assert!(stale_policy.is_empty());
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .config
            .media_generation,
        6
    );

    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":6,"stateRevision":7,"buffering":true}}}}"#,
        )
        .expect("matching report should be applied");
    assert!(runtime.room_playback_state(&room).paused);
}

#[test]
fn controller_and_quorum_policies_use_only_the_capable_session_cohort() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(10.0));
    for (client_id, username, capable) in [
        ("alice-client", "alice", true),
        ("bob-client", "bob", true),
        ("charlie-client", "charlie", true),
        ("legacy-client", "legacy", false),
    ] {
        runtime
            .handle_line(client_id, &hello(username, &room, capable))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":4,"policy":"pauseController","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("controller policy should configure");
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":4,"buffering":true}}}}"#,
        )
        .expect("non-controller report should succeed");
    assert!(!runtime.room_playback_state(&room).paused);
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":4,"buffering":true}}}}"#,
        )
        .expect("controller report should pause");
    assert!(runtime.room_playback_state(&room).paused);

    runtime.set_time_now_override_seconds(Some(11.0));
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":4,"policy":"quorum","quorumPercent":66,"debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("quorum policy should replace and resume old policy pause");
    assert!(!runtime.room_playback_state(&room).paused);
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":4,"buffering":true}}}}"#,
        )
        .expect("first quorum report should succeed");
    assert!(!runtime.room_playback_state(&room).paused);
    let quorum = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":4,"buffering":true}}}}"#,
        )
        .expect("second quorum report should pause");
    assert!(runtime.room_playback_state(&room).paused);
    let status = buffering_status(&quorum).expect("quorum report should publish status");
    assert_eq!(status.config.policy, RoomBufferingPolicy::Quorum);
    assert_eq!(status.eligible_clients, 3);
    assert_eq!(status.required_buffering_clients, 2);
}

#[test]
fn disconnect_clears_participant_pressure_and_controller_disconnect_fails_open() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_time_now_override_seconds(Some(20.0));
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":5,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("policy should configure");
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":5,"buffering":true}}}}"#,
        )
        .expect("participant should pause room");
    assert!(runtime.room_playback_state(&room).paused);
    let bob_disconnect = runtime
        .handle_transport_disconnect_fanout("bob-client")
        .expect("participant disconnect should reevaluate policy");
    assert!(!runtime.room_playback_state(&room).paused);
    assert!(recipient_has_pause(&bob_disconnect, "alice-client", false));

    runtime
        .handle_line("bob-2", &hello("bob", &room, true))
        .expect("replacement participant should join");
    runtime
        .handle_line_fanout(
            "bob-2",
            r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":5,"buffering":true}}}}"#,
        )
        .expect("replacement report should pause room");
    assert!(runtime.room_playback_state(&room).paused);
    let controller_disconnect = runtime
        .handle_transport_disconnect_fanout("alice-client")
        .expect("configurer disconnect should fail open");
    assert!(!runtime.room_playback_state(&room).paused);
    let status = buffering_status(&controller_disconnect)
        .expect("remaining capable client should receive disabled status");
    assert_eq!(status.config.policy, RoomBufferingPolicy::Independent);
    assert_eq!(status.phase, RoomBufferingPhase::Independent);
}
