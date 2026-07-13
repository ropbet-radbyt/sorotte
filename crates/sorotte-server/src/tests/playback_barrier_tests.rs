use super::*;
use crate::ServerCompatibilityFallback;
use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_client_core::{
    ClientEffect, ClientEffectSink, ClientRuntime, ClientSession, LogicalMediaId,
    MediaTransportKind, PlaybackBarrierStartConfig, QueuedRuntimeControl,
};
use sorotte_player_api::DisconnectedPlayer;
use sorotte_protocol::{
    MediaLoadIntent, MediaReadyPayload, PlaybackBarrierDegradedReason,
    PlaybackBarrierParticipantPhase, PlaybackBarrierPhase, PlaybackBarrierRecoveryDisposition,
    PlaybackBarrierRequestResultStatus, PlaybackBarrierSetExtension, PlaybackBarrierStateExtension,
    PlaybackBarrierStatusPayload, RoomBufferingPhase, RoomBufferingPolicy,
    RoomBufferingStatusPayload, SOROTTE_PLAYBACK_BARRIER_V1, StatePayload,
    TransportBufferingReportPayload, encode_message_line,
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

fn buffering_snapshot_for(
    lines: &[DirectedOutboundLine],
    recipient: &str,
) -> Option<PlaybackBarrierSetExtension> {
    messages(lines)
        .into_iter()
        .find_map(|(client_id, message)| {
            if client_id != recipient {
                return None;
            }
            let extension = barrier_extension(&message)?;
            (extension.buffering_policy.is_some() && extension.buffering_status.is_some())
                .then_some(extension)
        })
}

fn playback_barrier_status(lines: &[DirectedOutboundLine]) -> Option<PlaybackBarrierStatusPayload> {
    messages(lines)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .find_map(|extension| extension.status)
}

fn recovery_snapshot_for(
    lines: &[DirectedOutboundLine],
    recipient: &str,
) -> Option<PlaybackBarrierSetExtension> {
    messages(lines)
        .into_iter()
        .find_map(|(client_id, message)| {
            if client_id != recipient {
                return None;
            }
            let extension = barrier_extension(&message)?;
            extension.recovery.is_some().then_some(extension)
        })
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
fn coalesced_client_ready_and_initial_transport_report_both_reach_server() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("barrier-aware hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":41,"loadIntent":"newPlayback","logicalMediaId":"coalesced-observations","targetPosition":0.0,"policy":"allEligible","timeoutMs":5000},"bufferingPolicy":{"mediaGeneration":0,"requestNonce":41,"loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":750,"resumeHysteresisMs":1500,"maxPauseMs":5000}}}}"#,
        )
        .expect("combined start and buffering policy should configure");

    let mut client_outbox = QueuedRuntimeControl::default();
    client_outbox.begin_protocol_connection_generation();
    client_outbox.activate_protocol_connection_generation();
    client_outbox
        .emit(ClientEffect::SendState(
            StatePayload::new().with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new()
                    .with_ready(MediaReadyPayload::new(1, true, true).with_seekable(true)),
            ),
        ))
        .expect("readiness should queue");
    client_outbox
        .emit(ClientEffect::SendState(
            StatePayload::new().with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new()
                    .with_transport(TransportBufferingReportPayload::new(1, false)),
            ),
        ))
        .expect("initial transport report should queue");

    let coalesced = client_outbox.drain_outbound_messages();
    assert_eq!(coalesced.len(), 1, "the pending States should coalesce");
    let line = encode_message_line(&coalesced[0]).expect("coalesced State should encode");
    runtime
        .handle_line_fanout("alice-client", &line)
        .expect("server should accept both coalesced observations");

    let barrier = runtime
        .room_playback_barriers
        .get(&room)
        .expect("start barrier should remain active while bob is pending");
    assert_eq!(
        barrier.participants["alice-client"].status.phase,
        PlaybackBarrierParticipantPhase::Ready,
        "the readiness acknowledgement must survive State coalescing"
    );
    let buffering = runtime
        .room_buffering_controls
        .get(&room)
        .expect("room buffering policy should remain active");
    assert!(
        !buffering.reports["alice-client"].buffering,
        "the initial transport report must survive alongside readiness"
    );
}

#[test]
fn reconnect_rebuilds_undelivered_start_before_server_acceptance() {
    let mut server = ServerRuntime::default();
    server
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("server hello should succeed");

    let mut client_session = ClientSession::default();
    client_session
        .apply_message_json(&hello("alice", "room", true))
        .expect("client hello should apply");
    let mut client = ClientRuntime::new(
        client_session,
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    client.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(sorotte_protocol::PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    client.prepare_playback_media(
        LogicalMediaId::new("undelivered-server-media").unwrap(),
        MediaTransportKind::NetworkVod,
        1.0,
    );
    let ProtocolMessage::Set(initial) = &client.control().outbound_messages()[0] else {
        panic!("initial request should use a Set envelope");
    };
    let initial_prepare = initial
        .set
        .playback_barrier_v1()
        .expect("initial extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("initial start request should be present");
    let initial_nonce = initial_prepare.request_nonce;
    let initial_request_id = initial_prepare
        .request_id
        .expect("current clients should attach a stable operation identity");

    let pending = client
        .pending_protocol_line()
        .expect("initial request should encode")
        .expect("initial request should be staged for transport");
    let locally_written_line = pending.line().to_owned();
    client
        .acknowledge_protocol_line(pending.lease())
        .expect("successful local transport write should release the staged bytes");
    assert!(client.control().outbound_messages().is_empty());
    assert!(
        locally_written_line.contains("undelivered-server-media"),
        "the simulated transport write must own the original start request"
    );
    // Deliberately do not pass `locally_written_line` to the server. This is
    // the uncertain-delivery boundary: the socket write succeeded locally,
    // but the server application never parsed or accepted those bytes.

    // Starting a new connection generation must retain the semantic operation
    // even though its serialized bytes have already left the client outbox.
    client.begin_protocol_connection_generation();
    client.session_mut().mark_reconnecting(1);
    client.session_mut().reset_sync_state_for_reconnect();
    client
        .session_mut()
        .apply_message_json(&hello("alice", "room", true))
        .expect("replacement Hello should apply");
    client
        .run_controller_auth_notifications_if_needed()
        .expect("current intent should rebuild after replacement Hello");

    let recovery_query = client.flush_queued_protocol_messages();
    assert_eq!(recovery_query.len(), 1);
    let ProtocolMessage::Set(recovery_set) = &recovery_query[0] else {
        panic!("recovery query should use a Set envelope");
    };
    let recovery = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .and_then(|extension| extension.recovery)
        .expect("uncertain start should query the server before rebuilding");
    assert_eq!(recovery.original_request_nonce, initial_nonce);
    assert_eq!(recovery.request_id, initial_request_id);

    let recovery_line =
        encode_message_line(&recovery_query[0]).expect("recovery query should encode");
    let absent = server
        .handle_line_fanout("alice-client", &recovery_line)
        .expect("server should explicitly report the missing operation");
    for (recipient, message) in messages(&absent) {
        if recipient == "alice-client" {
            client
                .session_mut()
                .apply_protocol_message(message)
                .expect("client should apply the recovery result");
        }
    }
    client
        .run_controller_auth_notifications_if_needed()
        .expect("Absent should rearm exactly one fresh start");

    let rebuilt = client.flush_queued_protocol_messages();
    assert_eq!(rebuilt.len(), 1);
    let ProtocolMessage::Set(rebuilt_set) = &rebuilt[0] else {
        panic!("rebuilt request should use a Set envelope");
    };
    let rebuilt_prepare = rebuilt_set
        .set
        .playback_barrier_v1()
        .expect("rebuilt extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("Absent recovery should rebuild the semantic start");
    assert_eq!(rebuilt_prepare.request_nonce, initial_nonce);
    assert_eq!(
        rebuilt_prepare.request_id.as_deref(),
        Some(initial_request_id.as_str())
    );
    assert_eq!(rebuilt_prepare.load_intent, MediaLoadIntent::NewPlayback);

    let rebuilt_line = encode_message_line(&rebuilt[0]).expect("rebuilt request should encode");
    server
        .handle_line_fanout("alice-client", &rebuilt_line)
        .expect("server should accept the fresh semantic request");
    let barrier = server
        .room_playback_barriers
        .get("room")
        .expect("fresh request should create the room barrier");
    assert_eq!(barrier.prepare.request_nonce, rebuilt_prepare.request_nonce);
    assert_eq!(barrier.prepare.logical_media_id, "undelivered-server-media");
}

#[test]
fn client_runtime_recovers_server_processed_start_after_response_loss() {
    let mut server = ServerRuntime::default();
    server
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("initial transport should join");

    let mut client_session = ClientSession::default();
    client_session
        .apply_message_json(&hello("alice", "room", true))
        .expect("initial server Hello should apply");
    let mut client = ClientRuntime::new(
        client_session,
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    client.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(sorotte_protocol::PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });
    client.prepare_playback_media(
        LogicalMediaId::new("processed-lost-response-media").unwrap(),
        MediaTransportKind::NetworkVod,
        6.0,
    );

    let ProtocolMessage::Set(initial_set) = &client.control().outbound_messages()[0] else {
        panic!("initial request should use a Set envelope");
    };
    let initial_prepare = initial_set
        .set
        .playback_barrier_v1()
        .expect("initial extension should decode")
        .and_then(|extension| extension.prepare)
        .expect("initial start request should be present");
    let initial_nonce = initial_prepare.request_nonce;
    let initial_request_id = initial_prepare
        .request_id
        .expect("current clients should attach a stable operation identity");

    let pending = client
        .pending_protocol_line()
        .expect("initial request should encode")
        .expect("initial request should be staged for transport");
    let locally_written_line = pending.line().to_owned();
    client
        .acknowledge_protocol_line(pending.lease())
        .expect("successful local write should release the staged bytes");
    server
        .handle_line_fanout("alice-old", &locally_written_line)
        .expect("the server should process the request");
    // Drop the returned canonical prepare to model a disconnect after server
    // acceptance but before the application-level response reaches the client.
    assert_eq!(server.next_playback_barrier_generation, 1);
    assert_eq!(
        server.room_playback_barriers["room"]
            .prepare
            .request_id
            .as_deref(),
        Some(initial_request_id.as_str())
    );

    server
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement transport should join");
    client.begin_protocol_connection_generation();
    client.session_mut().mark_reconnecting(1);
    client.session_mut().reset_sync_state_for_reconnect();
    client
        .session_mut()
        .apply_message_json(&hello("alice", "room", true))
        .expect("replacement server Hello should apply");
    client
        .run_controller_auth_notifications_if_needed()
        .expect("replacement connection should emit recovery");

    let recovery_query = client.flush_queued_protocol_messages();
    assert_eq!(recovery_query.len(), 1);
    let ProtocolMessage::Set(recovery_set) = &recovery_query[0] else {
        panic!("recovery query should use a Set envelope");
    };
    let recovery = recovery_set
        .set
        .playback_barrier_v1()
        .expect("recovery extension should decode")
        .and_then(|extension| extension.recovery)
        .expect("accepted but unconfirmed start should be recovered first");
    assert_eq!(recovery.request_id, initial_request_id);
    assert_eq!(recovery.original_request_nonce, initial_nonce);

    let recovery_line =
        encode_message_line(&recovery_query[0]).expect("recovery query should encode");
    let recovered = server
        .handle_line_fanout("alice-new", &recovery_line)
        .expect("replacement should recover the accepted request");
    let recovered_extension = recovery_snapshot_for(&recovered, "alice-new")
        .expect("server should return the canonical lifecycle");
    assert_eq!(
        recovered_extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(
        recovered_extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );

    for (recipient, message) in messages(&recovered) {
        if recipient == "alice-new" {
            client
                .session_mut()
                .apply_protocol_message(message)
                .expect("client should apply the recovered canonical lifecycle");
        }
    }
    client
        .run_controller_auth_notifications_if_needed()
        .expect("canonical recovery should settle the semantic operation");

    let canonical_prepare = client
        .session()
        .playback_barrier_prepare()
        .expect("client session should retain the recovered canonical prepare");
    assert_eq!(canonical_prepare.media_generation, 1);
    assert_eq!(
        canonical_prepare.request_id.as_deref(),
        Some(initial_request_id.as_str())
    );
    let post_recovery = client.flush_queued_protocol_messages();
    assert!(
        post_recovery.iter().all(|message| {
            barrier_extension(message).is_none_or(|extension| extension.prepare.is_none())
        }),
        "a recovered canonical lifecycle must not emit a fresh or competing prepare"
    );
    assert_eq!(server.next_playback_barrier_generation, 1);
    assert_eq!(
        server.room_playback_barriers["room"]
            .prepare
            .media_generation,
        1
    );
}

#[test]
fn recovery_query_reports_absent_before_a_transport_written_request_is_processed() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("hello should succeed");

    let response = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"write-only-operation","originalRequestNonce":11,"recoveryNonce":12,"logicalMediaId":"write-only-media"}}}}"#,
        )
        .expect("recovery query should succeed");
    let extension = recovery_snapshot_for(&response, "alice-client")
        .expect("an explicit negative recovery response is required");
    let recovery = extension
        .recovery
        .expect("recovery result should be present");
    assert_eq!(
        recovery.disposition,
        Some(PlaybackBarrierRecoveryDisposition::Absent)
    );
    assert_eq!(recovery.media_generation, None);
    assert!(extension.prepare.is_none());
    assert_eq!(runtime.next_playback_barrier_generation, 0);
}

#[test]
fn recovery_distinguishes_existing_superseded_and_rejected_queries() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":15,"requestId":"current-operation","loadIntent":"newPlayback","logicalMediaId":"shared-media","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("current lifecycle should start");

    let existing = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"other-operation","originalRequestNonce":1,"recoveryNonce":2,"logicalMediaId":"shared-media"}}}}"#,
        )
        .expect("same-media lifecycle should be reported as existing");
    let existing = recovery_snapshot_for(&existing, "bob-client")
        .expect("existing response should include the canonical snapshot");
    assert_eq!(
        existing
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Existing)
    );
    assert!(existing.prepare.is_some());
    assert_eq!(
        existing
            .prepare
            .as_ref()
            .and_then(|prepare| prepare.request_id.as_deref()),
        None,
        "the owner's bearer-style request identity must not leak in a peer snapshot"
    );

    let superseded = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"superseded-operation","originalRequestNonce":3,"recoveryNonce":4,"logicalMediaId":"old-media"}}}}"#,
        )
        .expect("different-media lifecycle should be reported as superseded");
    let superseded = recovery_snapshot_for(&superseded, "bob-client")
        .expect("superseded response should be explicit");
    assert_eq!(
        superseded
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Superseded)
    );
    assert!(superseded.prepare.is_none());

    let rejected = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"invalid-operation","originalRequestNonce":5,"recoveryNonce":0,"logicalMediaId":"shared-media"}}}}"#,
        )
        .expect("invalid recovery query should receive a rejection");
    assert_eq!(
        recovery_snapshot_for(&rejected, "bob-client")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Rejected)
    );

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("controller readiness should commit the lifecycle");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":0.0}}}}"#,
        )
        .expect("controller StartedAck should succeed");
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":0.0}}}}"#,
        )
        .expect("remaining participant StartedAck should complete the lifecycle");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Complete
    );

    let exact_terminal = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"current-operation","originalRequestNonce":15,"recoveryNonce":6,"logicalMediaId":"shared-media"}}}}"#,
        )
        .expect("exact operation should recover retained terminal diagnostics");
    let exact_terminal = recovery_snapshot_for(&exact_terminal, "alice-client")
        .expect("exact terminal recovery should include its snapshot");
    assert_eq!(
        exact_terminal
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(
        exact_terminal.status.as_ref().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Complete)
    );

    let different_terminal = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"new-replay-operation","originalRequestNonce":7,"recoveryNonce":8,"logicalMediaId":"shared-media"}}}}"#,
        )
        .expect("different operation must not be suppressed by terminal history");
    let different_terminal = recovery_snapshot_for(&different_terminal, "bob-client")
        .expect("terminal history should produce an explicit Absent result");
    assert_eq!(
        different_terminal
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Absent)
    );
    assert!(different_terminal.prepare.is_none());
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

#[test]
fn processed_start_with_lost_response_recovers_the_existing_generation() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-old", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":21,"requestId":"lost-response-operation","loadIntent":"newPlayback","logicalMediaId":"lost-response-media","targetPosition":4.0,"policy":"controller"}}}}"#,
        )
        .expect("the server should process the request even though its response is lost");
    assert_eq!(runtime.next_playback_barrier_generation, 1);

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("old transport should disconnect");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement transport should join");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"lost-response-operation","originalRequestNonce":21,"recoveryNonce":22,"logicalMediaId":"lost-response-media"}}}}"#,
        )
        .expect("replacement should recover the accepted request");
    let extension = recovery_snapshot_for(&recovered, "alice-new")
        .expect("recovery should return the full canonical lifecycle");
    assert_eq!(
        extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(
        extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.media_generation),
        Some(1)
    );
    assert_eq!(
        extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.media_generation),
        Some(1)
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.initiator_client_id, "alice-new");
    assert!(barrier.participants.contains_key("alice-new"));
    assert!(!barrier.participants.contains_key("alice-old"));

    let retry = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"lost-response-operation","originalRequestNonce":21,"recoveryNonce":22,"logicalMediaId":"lost-response-media"}}}}"#,
        )
        .expect("an exact recovery retry should be idempotent");
    assert_eq!(
        recovery_snapshot_for(&retry, "alice-new")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

#[test]
fn reconnect_during_committed_before_started_ack_restores_and_completes_lifecycle() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username, capable) in [
        ("alice-old", "alice", true),
        ("legacy-client", "legacy", false),
    ] {
        runtime
            .handle_line(client_id, &hello(username, "room", capable))
            .expect("hello should succeed");
    }
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":31,"requestId":"committed-operation","loadIntent":"newPlayback","logicalMediaId":"committed-media","targetPosition":8.0,"policy":"controller"}}}}"#,
        )
        .expect("prepare should succeed");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("controller readiness should commit");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("disconnect before StartedAck should be recorded");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should join");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"committed-operation","originalRequestNonce":31,"recoveryNonce":32,"logicalMediaId":"committed-media"}}}}"#,
        )
        .expect("committed lifecycle should recover");
    let extension = recovery_snapshot_for(&recovered, "alice-new")
        .expect("recovery should include the committed lifecycle");
    assert_eq!(
        extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert!(extension.prepare.is_some());
    assert!(extension.commit.is_some());
    assert_eq!(
        extension.status.as_ref().map(|status| status.phase),
        Some(PlaybackBarrierPhase::Committed)
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].participants["alice-new"]
            .status
            .phase,
        PlaybackBarrierParticipantPhase::Ready
    );

    let completed = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":8.1}}}}"#,
        )
        .expect("replacement StartedAck should be accepted");
    let status = playback_barrier_status(&completed).expect("completion status should publish");
    assert_eq!(status.phase, PlaybackBarrierPhase::Complete);
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert_eq!(runtime.next_playback_barrier_revision, 1);
}

#[test]
fn recovery_rebinds_and_fences_old_connection_that_is_still_present() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old connection should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":41,"requestId":"overlap-operation","loadIntent":"newPlayback","logicalMediaId":"overlap-media","targetPosition":2.0,"policy":"controller"},"bufferingPolicy":{"mediaGeneration":0,"requestNonce":41,"requestId":"overlap-operation","loadIntent":"newPlayback","policy":"independent","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("old connection should establish the lifecycle");

    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should join before the old transport disappears");
    assert_eq!(runtime.sessions["alice-new"].username, "alice_");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"overlap-operation","originalRequestNonce":41,"recoveryNonce":42,"logicalMediaId":"overlap-media"}}}}"#,
        )
        .expect("newer connection should recover and fence the old identity");
    assert_eq!(
        recovery_snapshot_for(&recovered, "alice-new")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.initiator_client_id, "alice-new");
    assert_eq!(barrier.initiator_username, "alice_");
    assert_eq!(barrier.participants.len(), 1);
    assert!(barrier.participants.contains_key("alice-new"));
    assert!(!barrier.participants.contains_key("alice-old"));
    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "alice-new"
    );

    let stale_ready = runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("fenced transport input should be safely ignored");
    assert!(stale_ready.is_empty());
    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("late old-transport disconnect should be inert");
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.initiator_client_id, "alice-new");
    assert!(barrier.participants.contains_key("alice-new"));
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "alice-new"
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

fn runtime_with_recovered_overlapping_committed_owner() -> ServerRuntime {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(50.0));
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old connection should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"file":{"name":"revoked-owner.mkv","duration":120.0}}}"#,
        )
        .expect("old connection should publish its media");
    let initial_sync_counter = runtime.server_ignoring_counter("alice-old");
    runtime.acknowledge_server_ignoring_counter("alice-old", initial_sync_counter);
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"playstate":{"position":2.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should begin in a playing state");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":141,"requestId":"revoked-transport-operation","loadIntent":"newPlayback","logicalMediaId":"revoked-transport-media","targetPosition":2.0,"policy":"controller"},"bufferingPolicy":{"mediaGeneration":0,"requestNonce":141,"requestId":"revoked-transport-operation","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("old connection should start a lifecycle");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("owner readiness should commit the lifecycle");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("new connection should overlap the old transport");
    let recovery = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"revoked-transport-operation","originalRequestNonce":141,"recoveryNonce":142,"logicalMediaId":"revoked-transport-media"}}}}"#,
        )
        .expect("new connection should recover the committed lifecycle");
    assert!(has_close_transport_action(
        &recovery.transport_actions,
        "alice-old"
    ));
    assert_eq!(
        recovery_snapshot_for(&recovery.outbound_lines, "alice-new")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    runtime
}

#[test]
fn fenced_old_transport_cannot_pause_or_seek_after_recovery() {
    let mut runtime = runtime_with_recovered_overlapping_committed_owner();
    let old_counter = runtime.server_ignoring_counter("alice-old");
    runtime.acknowledge_server_ignoring_counter("alice-old", old_counter);
    let state_before = runtime.room_playback_state("room");

    let pause = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-old",
            r#"{"State":{"playstate":{"position":2.0,"paused":true,"doSeek":false}}}"#,
        )
        .expect("fenced pause should be safely rejected");
    assert!(pause.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &pause.transport_actions,
        "alice-old"
    ));
    assert_eq!(runtime.room_playback_state("room"), state_before);

    let old_counter = runtime.server_ignoring_counter("alice-old");
    runtime.acknowledge_server_ignoring_counter("alice-old", old_counter);

    let seek = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-old",
            r#"{"State":{"playstate":{"position":99.0,"paused":false,"doSeek":true}}}"#,
        )
        .expect("fenced seek should be safely rejected");
    assert!(seek.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &seek.transport_actions,
        "alice-old"
    ));
    assert_eq!(runtime.room_playback_state("room"), state_before);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
}

#[test]
fn fenced_old_transport_cannot_retire_awaiting_controller_decision() {
    let mut runtime = ServerRuntime::default();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old connection should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("room should start playing");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":151,"requestId":"awaiting-owner-operation","loadIntent":"newPlayback","logicalMediaId":"awaiting-owner-media","targetPosition":4.0,"policy":"controller","timeoutMs":1000,"timeoutAction":"askController"}}}}"#,
        )
        .expect("prepare should succeed");
    runtime
        .collect_dispatch_at(101.0)
        .expect("prepare should enter AwaitingDecision");
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::AwaitingDecision
    );
    let forced_pause_counter = runtime.server_ignoring_counter("alice-old");
    runtime.acknowledge_server_ignoring_counter("alice-old", forced_pause_counter);

    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("new connection should overlap old owner");
    let recovery = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"awaiting-owner-operation","originalRequestNonce":151,"recoveryNonce":152,"logicalMediaId":"awaiting-owner-media"}}}}"#,
        )
        .expect("new connection should recover awaiting lifecycle");
    assert!(has_close_transport_action(
        &recovery.transport_actions,
        "alice-old"
    ));

    let stale_decision = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-old",
            r#"{"State":{"playstate":{"position":4.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("fenced controller decision should be rejected");
    assert!(stale_decision.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &stale_decision.transport_actions,
        "alice-old"
    ));
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::AwaitingDecision
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].initiator_client_id,
        "alice-new"
    );
}

#[test]
fn fenced_old_transport_cannot_rehello_and_reclaim_operation() {
    let mut runtime = runtime_with_recovered_overlapping_committed_owner();
    let old_join_sequence = runtime.client_room_join_sequence["alice-old"];
    let new_join_sequence = runtime.client_room_join_sequence["alice-new"];

    let repeated_hello = runtime
        .handle_line_fanout_with_transport_actions("alice-old", &hello("alice", "room", true))
        .expect("fenced repeated Hello should be safely rejected");
    assert!(repeated_hello.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &repeated_hello.transport_actions,
        "alice-old"
    ));
    assert_eq!(
        runtime.client_room_join_sequence["alice-old"],
        old_join_sequence
    );
    assert_eq!(
        runtime.client_room_join_sequence["alice-new"],
        new_join_sequence
    );
    assert_eq!(runtime.sessions["alice-old"].username, "alice");

    let replay = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":141,"requestId":"revoked-transport-operation","loadIntent":"newPlayback","logicalMediaId":"revoked-transport-media","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .expect("fenced operation replay should be safely rejected");
    assert!(replay.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &replay.transport_actions,
        "alice-old"
    ));
    assert_eq!(
        runtime.room_playback_barriers["room"].initiator_client_id,
        "alice-new"
    );
    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
    assert!(
        !runtime
            .playback_barrier_fenced_clients
            .contains("alice-new")
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

#[test]
fn fenced_old_transport_disconnect_is_inert_after_recovery() {
    let mut runtime = runtime_with_recovered_overlapping_committed_owner();
    let barrier_before = runtime.room_playback_barriers["room"].clone();
    let buffering_owner_before = runtime.room_buffering_controls["room"]
        .configured_by_client_id
        .clone();

    let disconnected = runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("eventual old transport disconnect should clean up safely");
    assert!(playback_barrier_status(&disconnected).is_none());
    assert_eq!(runtime.room_playback_barriers["room"], barrier_before);
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        buffering_owner_before
    );
    assert!(!runtime.sessions.contains_key("alice-old"));
    assert!(runtime.sessions.contains_key("alice-new"));
    assert!(
        !runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

#[test]
fn fenced_disconnect_cleans_current_room_without_touching_recovered_room() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room-a", true))
        .expect("old owner should join room A");
    runtime
        .handle_line("charlie-a", &hello("charlie", "room-a", true))
        .expect("room A peer should retain its lifecycle while the old owner moves");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":201,"requestId":"multi-room-operation-a","loadIntent":"newPlayback","logicalMediaId":"multi-room-media-a","targetPosition":1.0,"policy":"controller"}}}}"#,
        )
        .expect("room A lifecycle should start");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":1,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("room A lifecycle should commit");
    runtime
        .handle_line("bob", &hello("bob", "room-b", true))
        .expect("room B peer should keep that room live");
    runtime
        .handle_line_fanout("alice-old", r#"{"Set":{"room":{"name":"room-b"}}}"#)
        .expect("old transport should move to room B before recovery");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":202,"requestId":"multi-room-operation-b","loadIntent":"newPlayback","logicalMediaId":"multi-room-media-b","targetPosition":2.0,"policy":"controller"}}}}"#,
        )
        .expect("old transport should start an unrelated room B lifecycle");
    assert!(
        runtime.room_playback_barriers["room-b"]
            .participants
            .contains_key("alice-old")
    );

    runtime
        .handle_line("alice-new", &hello("alice", "room-a", true))
        .expect("replacement should join room A");
    let recovery = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"multi-room-operation-a","originalRequestNonce":201,"recoveryNonce":203,"logicalMediaId":"multi-room-media-a"}}}}"#,
        )
        .expect("room A lifecycle should recover to the replacement");
    assert!(has_close_transport_action(
        &recovery.transport_actions,
        "alice-old"
    ));
    let recovered_room_a = runtime.room_playback_barriers["room-a"].clone();
    assert_eq!(recovered_room_a.initiator_client_id, "alice-new");

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("physical disconnect should clean the transport's current room");
    assert_eq!(
        runtime.room_playback_barriers["room-a"], recovered_room_a,
        "room B disconnect cleanup must remain inert for recovered room A"
    );
    assert_eq!(
        runtime.room_playback_barriers["room-b"].participants["alice-old"]
            .status
            .degraded_reason,
        Some(PlaybackBarrierDegradedReason::Disconnected),
        "global fencing must not suppress cleanup of an unrelated current-room lifecycle"
    );
}

#[test]
fn fenced_transport_cached_sample_is_excluded_from_periodic_room_derivation() {
    let mut runtime = runtime_with_recovered_overlapping_committed_owner();
    runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"file":{"name":"revoked-owner.mkv","duration":120.0}}}"#,
        )
        .expect("replacement should publish the same media");
    let replacement_counter = runtime.server_ignoring_counter("alice-new");
    runtime.acknowledge_server_ignoring_counter("alice-new", replacement_counter);
    runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false}}}"#,
        )
        .expect("replacement should publish a current playback sample");

    let (set_by, position) = runtime
        .slowest_room_playback_client_at("room", false, 50.0)
        .expect("replacement sample should remain eligible");
    assert_eq!(set_by, "alice_");
    assert_eq!(position, 10.0);
    assert_eq!(
        runtime.client_playback_states["alice-old"].position,
        Some(2.0),
        "the stale sample remains cached until physical disconnect, making the exclusion material"
    );
}

#[test]
fn fenced_transport_periodic_timeout_keeps_fence_through_rehello_race() {
    let mut runtime = runtime_with_recovered_overlapping_committed_owner();
    let old_join_sequence = runtime.client_room_join_sequence["alice-old"];
    runtime
        .client_last_state_update_at
        .insert("alice-old".to_owned(), 0.0);
    assert!(runtime.client_timed_out("alice-old", 100.0));

    let periodic = runtime
        .collect_periodic_tick_for_client_at("alice-old", 100.0, 100.0)
        .expect("fenced periodic tick should remain inert");
    assert!(periodic.is_empty());
    assert!(has_close_transport_action(
        &runtime.drain_transport_actions(),
        "alice-old"
    ));
    assert!(runtime.sessions.contains_key("alice-old"));
    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );

    let racing_hello = runtime
        .handle_line_fanout_with_transport_actions("alice-old", &hello("alice", "room", true))
        .expect("racing Hello should be rejected while teardown is pending");
    assert!(racing_hello.outbound_lines.is_empty());
    assert!(has_close_transport_action(
        &racing_hello.transport_actions,
        "alice-old"
    ));
    assert_eq!(
        runtime.client_room_join_sequence["alice-old"],
        old_join_sequence
    );
    assert_eq!(
        runtime.room_playback_barriers["room"].initiator_client_id,
        "alice-new"
    );

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("physical disconnect should finally remove fenced model state");
    assert!(!runtime.sessions.contains_key("alice-old"));
    assert!(
        !runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
}

#[test]
fn same_request_id_replayed_from_newer_connection_is_cross_connection_idempotent() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old connection should join");
    let request = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":51,"requestId":"idempotent-operation","loadIntent":"newPlayback","logicalMediaId":"idempotent-media","targetPosition":0.0,"policy":"controller"}}}}"#;
    runtime
        .handle_line_fanout("alice-old", request)
        .expect("initial request should succeed");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should overlap the old connection");

    let retry = runtime
        .handle_line_fanout("alice-new", request)
        .expect("same application operation should replay canonically");
    let extension = messages(&retry)
        .into_iter()
        .find_map(|(recipient, message)| {
            (recipient == "alice-new")
                .then(|| barrier_extension(&message))
                .flatten()
                .filter(|extension| extension.prepare.is_some())
        })
        .expect("replacement should receive the existing canonical prepare");
    assert_eq!(
        extension.prepare.map(|prepare| prepare.media_generation),
        Some(1)
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert_eq!(
        runtime.room_playback_barriers["room"].initiator_client_id,
        "alice-new"
    );
}

#[test]
fn delayed_accepted_operation_cannot_resurrect_after_a_replacement_generation() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("controller should join");
    let operation_a = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":71,"requestId":"accepted-operation-a","loadIntent":"newPlayback","logicalMediaId":"media-a","targetPosition":0.0,"policy":"controller"}}}}"#;
    runtime
        .handle_line_fanout("alice-old", operation_a)
        .expect("operation A should allocate generation one");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":72,"requestId":"replacement-operation-b","loadIntent":"newPlayback","logicalMediaId":"media-b","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("operation B should supersede A with generation two");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .request_id
            .as_deref(),
        Some("replacement-operation-b")
    );

    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("newer transport should overlap the original");
    let delayed = runtime
        .handle_line_fanout("alice-new", operation_a)
        .expect("delayed accepted frame should be safely consumed");
    assert!(delayed.is_empty());
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .request_id
            .as_deref(),
        Some("replacement-operation-b")
    );

    let recovery = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"accepted-operation-a","originalRequestNonce":71,"recoveryNonce":73,"logicalMediaId":"media-a"}}}}"#,
        )
        .expect("stale accepted operation should have an explicit tombstone result");
    let recovery = recovery_snapshot_for(&recovery, "alice-new")
        .and_then(|extension| extension.recovery)
        .expect("tombstoned operation should receive a result");
    assert_eq!(
        recovery.disposition,
        Some(PlaybackBarrierRecoveryDisposition::Superseded)
    );
    assert_eq!(recovery.media_generation, Some(1));
    assert_eq!(runtime.next_playback_barrier_generation, 2);
}

#[test]
fn policy_only_operation_recovers_and_rebinds_without_allocating_a_start_barrier() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old policy owner should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":61,"requestId":"policy-only-operation","loadIntent":"newPlayback","policy":"independent","debounceMs":100,"resumeHysteresisMs":200,"maxPauseMs":5000}}}}"#,
        )
        .expect("policy-only operation should configure");
    assert!(!runtime.room_playback_barriers.contains_key("room"));
    assert_eq!(runtime.next_playback_barrier_generation, 1);

    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should overlap the old owner");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"policy-only-operation","originalRequestNonce":61,"recoveryNonce":62,"logicalMediaId":"policy-only-media"}}}}"#,
        )
        .expect("policy-only recovery should succeed");
    let extension = recovery_snapshot_for(&recovered, "alice-new")
        .expect("policy-only recovery should be explicit");
    assert_eq!(
        extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert!(extension.prepare.is_none());
    assert!(extension.status.is_none());
    assert_eq!(
        extension
            .buffering_policy
            .as_ref()
            .map(|policy| policy.media_generation),
        Some(1)
    );
    assert!(extension.buffering_status.is_some());
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "alice-new"
    );
    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("late old-owner disconnect should not disable replacement policy");
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "alice-new"
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
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
fn active_barriers_are_superseded_only_by_the_same_session_with_a_higher_nonce() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, "room", true))
            .expect("hello should succeed");
    }
    let first_request = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","logicalMediaId":"media-a","targetPosition":0.0,"policy":"allEligible"}}}}"#;
    runtime
        .handle_line_fanout("alice-client", first_request)
        .expect("media A should start preparing");

    let other_controller = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":20,"loadIntent":"newPlayback","logicalMediaId":"media-b","targetPosition":4.0,"policy":"allEligible"}}}}"#,
        )
        .expect("another controller's replacement should be safely rejected");
    assert!(other_controller.is_empty());
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .media_generation,
        1
    );

    let replacement = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"newPlayback","logicalMediaId":"media-b","targetPosition":4.0,"policy":"allEligible"}}}}"#,
        )
        .expect("the initiating session should supersede its preparing barrier");
    let replacement_extensions: Vec<_> = messages(&replacement)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .collect();
    let superseded = replacement_extensions
        .iter()
        .filter_map(|extension| extension.status.as_ref())
        .find(|status| status.media_generation == 1)
        .expect("participants should receive terminal status for media A");
    assert_eq!(superseded.phase, PlaybackBarrierPhase::Degraded);
    assert!(superseded.participants.values().all(|participant| {
        participant.phase == PlaybackBarrierParticipantPhase::Degraded
            && participant.degraded_reason == Some(PlaybackBarrierDegradedReason::Superseded)
    }));
    assert!(replacement_extensions.iter().any(|extension| {
        extension.prepare.as_ref().is_some_and(|prepare| {
            prepare.media_generation == 2
                && prepare.request_nonce == 2
                && prepare.logical_media_id == "media-b"
        })
    }));

    let stale_first = runtime
        .handle_line_fanout("alice-client", first_request)
        .expect("a stale media A retry should be safely ignored");
    assert!(stale_first.is_empty());
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .media_generation,
        2
    );

    for client_id in ["alice-client", "bob-client"] {
        runtime
            .handle_line_fanout(
                client_id,
                r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":2,"loaded":true,"bufferReady":true}}}}"#,
            )
            .expect("media B readiness should succeed");
    }
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );

    let replay = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":3,"loadIntent":"replay","logicalMediaId":"media-b","targetPosition":0.0,"policy":"allEligible"}}}}"#,
        )
        .expect("the owner should supersede a committed barrier before StartedAck completion");
    let replay_extensions: Vec<_> = messages(&replay)
        .into_iter()
        .filter_map(|(_, message)| barrier_extension(&message))
        .collect();
    let superseded_commit = replay_extensions
        .iter()
        .filter_map(|extension| extension.status.as_ref())
        .find(|status| status.media_generation == 2)
        .expect("participants should receive terminal status for the committed generation");
    assert_eq!(superseded_commit.phase, PlaybackBarrierPhase::Degraded);
    assert!(superseded_commit.participants.values().all(|participant| {
        participant.degraded_reason == Some(PlaybackBarrierDegradedReason::Superseded)
    }));
    assert!(replay_extensions.iter().any(|extension| {
        extension.prepare.as_ref().is_some_and(|prepare| {
            prepare.media_generation == 3
                && prepare.request_nonce == 3
                && prepare.load_intent == MediaLoadIntent::Replay
        })
    }));
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
fn late_join_and_reconnect_receive_active_pause_any_and_quorum_snapshots() {
    for (policy_name, expected_policy) in [
        ("pauseAnyEligible", RoomBufferingPolicy::PauseAnyEligible),
        ("quorum", RoomBufferingPolicy::Quorum),
    ] {
        let room = controlled_room_name_for_test("room", "AB-123-456");
        let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
        runtime
            .handle_line("alice-client", &hello("alice", &room, true))
            .expect("controller hello should succeed");
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
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"bufferingPolicy":{{"mediaGeneration":7,"policy":"{policy_name}","quorumPercent":50,"debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}}}}}"#
                ),
            )
            .expect("active buffering policy should configure");

        let late_join = runtime
            .handle_line_fanout("bob-client", &hello("bob", &room, true))
            .expect("late join should succeed");
        let decoded = messages(&late_join);
        let hello_index = decoded
            .iter()
            .position(|(recipient, message)| {
                recipient == "bob-client" && matches!(message, ProtocolMessage::Hello(_))
            })
            .expect("late joiner should receive Hello");
        let snapshot_index = decoded
            .iter()
            .position(|(recipient, message)| {
                recipient == "bob-client"
                    && barrier_extension(message).is_some_and(|extension| {
                        extension.buffering_policy.is_some() && extension.buffering_status.is_some()
                    })
            })
            .expect("late joiner should receive active policy and status");
        assert!(snapshot_index > hello_index, "snapshot must follow Hello");
        let snapshot = buffering_snapshot_for(&late_join, "bob-client")
            .expect("late joiner should receive a complete snapshot");
        assert_eq!(
            snapshot
                .buffering_policy
                .as_ref()
                .map(|policy| policy.policy),
            Some(expected_policy)
        );
        assert_eq!(
            snapshot
                .buffering_status
                .as_ref()
                .map(|status| status.eligible_clients),
            Some(2)
        );

        runtime
            .handle_line_fanout(
                "bob-client",
                r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{"mediaGeneration":7,"buffering":true}}}}"#,
            )
            .expect("late joiner's current transport report should be accepted");
        assert!(runtime.room_playback_state(&room).paused);
        let reconnect = runtime
            .handle_line_fanout("bob-client", &hello("bob", &room, true))
            .expect("reconnect should succeed");
        assert!(
            !runtime.room_playback_state(&room).paused,
            "the old transport's buffering pressure must not survive reconnect"
        );
        let snapshot = buffering_snapshot_for(&reconnect, "bob-client")
            .expect("reconnected participant should receive the active snapshot");
        assert_eq!(
            snapshot
                .buffering_policy
                .as_ref()
                .map(|policy| policy.policy),
            Some(expected_policy)
        );
        assert_eq!(
            snapshot
                .buffering_status
                .as_ref()
                .map(|status| status.eligible_clients),
            Some(2)
        );
    }
}

#[test]
fn room_switch_and_capability_upgrade_receive_active_buffering_snapshot() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime
        .handle_line("alice-client", &hello("alice", &room, true))
        .expect("controller hello should succeed");
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":9,"policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("active policy should configure");

    runtime
        .handle_line("bob-client", &hello("bob", "lobby", true))
        .expect("bob should join another room");
    let switch = runtime
        .handle_line_fanout(
            "bob-client",
            &format!(r#"{{"Set":{{"room":{{"name":"{room}"}}}}}}"#),
        )
        .expect("room switch should succeed");
    assert!(
        buffering_snapshot_for(&switch, "bob-client").is_some(),
        "capable room switcher should receive the destination policy and status"
    );

    runtime
        .handle_line("charlie-client", &hello("charlie", &room, false))
        .expect("legacy-capability hello should succeed");
    let upgraded = runtime
        .handle_line_fanout(
            "charlie-client",
            r#"{"Set":{"features":{"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("capability upgrade should succeed");
    let snapshot = buffering_snapshot_for(&upgraded, "charlie-client")
        .expect("newly capable participant should receive policy and status");
    assert_eq!(
        snapshot
            .buffering_status
            .as_ref()
            .map(|status| status.eligible_clients),
        Some(3)
    );
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

#[test]
fn authenticated_policy_owner_reconnect_restores_coordination_with_fresh_nonce() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    for (client_id, username) in [("alice-client", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("hello should succeed");
    }
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":5,"requestNonce":1,"loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":250,"resumeHysteresisMs":750,"maxPauseMs":5000}}}}"#,
        )
        .expect("policy owner should configure coordinated buffering");
    let generation_before_reconnect = runtime.next_playback_barrier_generation;

    runtime
        .handle_transport_disconnect_fanout("alice-client")
        .expect("owner disconnect should fail open");
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::Independent
    );

    authenticate_policy_controller(&mut runtime, "bob-client");
    let replacement_controller = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":50,"loadIntent":"transportRefresh","policy":"quorum","quorumPercent":50,"debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":5000}}}}"#,
        )
        .expect("another authenticated controller's refresh should be safely rejected");
    assert!(replacement_controller.is_empty());
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::Independent,
        "controller authorization alone must not transfer policy ownership"
    );
    let forged_canonical_refresh = runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":5,"requestNonce":51,"loadIntent":"transportRefresh","policy":"pauseController"}}}}"#,
        )
        .expect("a refresh cannot bypass ownership with a client-supplied generation");
    assert!(forged_canonical_refresh.is_empty());

    runtime
        .handle_line("alice-reconnected", &hello("alice", &room, true))
        .expect("policy owner should reconnect");
    authenticate_policy_controller(&mut runtime, "alice-reconnected");
    let refresh = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":2,"loadIntent":"transportRefresh","policy":"pauseAnyEligible","debounceMs":250,"resumeHysteresisMs":750,"maxPauseMs":5000}}}}"#;
    let restored = runtime
        .handle_line_fanout("alice-reconnected", refresh)
        .expect("fresh authenticated owner intent should restore the policy");
    let snapshot = buffering_snapshot_for(&restored, "alice-reconnected")
        .expect("the restored canonical policy should be acknowledged");
    let restored_policy = snapshot
        .buffering_policy
        .expect("restoration snapshot should contain policy");
    assert_eq!(
        restored_policy.policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
    assert_eq!(restored_policy.media_generation, 5);
    assert_eq!(restored_policy.request_nonce, 2);
    assert_eq!(
        restored_policy.load_intent,
        MediaLoadIntent::TransportRefresh
    );
    let control = &runtime.room_buffering_controls[&room];
    assert_eq!(control.configured_by_client_id, "alice-reconnected");
    assert_eq!(control.configured_by_username, "alice");
    assert_eq!(
        runtime.next_playback_barrier_generation, generation_before_reconnect,
        "policy restoration must not allocate a new media barrier generation"
    );
    assert!(!runtime.room_playback_barriers.contains_key(&room));

    let retry = runtime
        .handle_line_fanout("alice-reconnected", refresh)
        .expect("an exact fresh-nonce retry should replay the canonical snapshot");
    assert!(buffering_snapshot_for(&retry, "alice-reconnected").is_some());
    let stale = runtime
        .handle_line_fanout(
            "alice-reconnected",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"transportRefresh","policy":"pauseController"}}}}"#,
        )
        .expect("older reconnect intent should be safely ignored");
    assert!(stale.is_empty());
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
}

#[test]
fn superseded_policy_operation_cannot_recover_or_replay_over_its_replacement() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime
        .handle_line("alice-old", &hello("alice", &room, true))
        .expect("policy owner should join");
    authenticate_policy_controller(&mut runtime, "alice-old");
    let operation_a = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":81,"requestId":"policy-operation-a","loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":100,"resumeHysteresisMs":200,"maxPauseMs":5000}}}}"#;
    runtime
        .handle_line_fanout("alice-old", operation_a)
        .expect("policy operation A should allocate generation one");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":82,"requestId":"policy-operation-b","loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":300,"resumeHysteresisMs":400,"maxPauseMs":5000}}}}"#,
        )
        .expect("policy operation B should replace A");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .config
            .request_id
            .as_deref(),
        Some("policy-operation-b")
    );

    runtime
        .handle_line("alice-new", &hello("alice", &room, true))
        .expect("replacement transport should overlap the original");
    authenticate_policy_controller(&mut runtime, "alice-new");
    let delayed = runtime
        .handle_line_fanout("alice-new", operation_a)
        .expect("a delayed policy A frame should be safely consumed");
    assert!(delayed.is_empty());
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .config
            .request_id
            .as_deref(),
        Some("policy-operation-b")
    );

    let recovery = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"policy-operation-a","originalRequestNonce":81,"recoveryNonce":83,"logicalMediaId":"policy-media-a"}}}}"#,
        )
        .expect("superseded policy recovery should return a terminal result");
    let recovery = recovery_snapshot_for(&recovery, "alice-new")
        .and_then(|extension| extension.recovery)
        .expect("superseded policy operation should receive a result");
    assert_eq!(
        recovery.disposition,
        Some(PlaybackBarrierRecoveryDisposition::Superseded)
    );
    assert_eq!(recovery.media_generation, Some(1));
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .config
            .request_id
            .as_deref(),
        Some("policy-operation-b")
    );
}

#[test]
fn policy_recovery_after_owner_disconnect_restores_the_requested_coordinated_policy() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    for (client_id, username) in [("alice-old", "alice"), ("bob-client", "bob")] {
        runtime
            .handle_line(client_id, &hello(username, &room, true))
            .expect("participant should join the controlled room");
    }
    authenticate_policy_controller(&mut runtime, "alice-old");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":91,"requestId":"restored-policy-operation","loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":250,"resumeHysteresisMs":750,"maxPauseMs":5000}}}}"#,
        )
        .expect("authenticated owner should configure coordinated buffering");
    let generation = runtime.next_playback_barrier_generation;
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .requested_config
            .policy,
        RoomBufferingPolicy::PauseAnyEligible
    );

    runtime
        .handle_transport_disconnect_fanout("alice-old")
        .expect("owner disconnect should fail open");
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::Independent
    );
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .requested_config
            .policy,
        RoomBufferingPolicy::PauseAnyEligible,
        "fail-open must retain the requested policy for application-level recovery"
    );

    runtime
        .handle_line("alice-new", &hello("alice", &room, true))
        .expect("policy owner should reconnect");
    authenticate_policy_controller(&mut runtime, "alice-new");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"restored-policy-operation","originalRequestNonce":91,"recoveryNonce":92,"logicalMediaId":"restored-policy-media"}}}}"#,
        )
        .expect("authenticated replacement should recover the policy operation");
    let extension = recovery_snapshot_for(&recovered, "alice-new")
        .expect("policy recovery should return the canonical snapshot");
    assert_eq!(
        extension
            .recovery
            .as_ref()
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(
        extension
            .buffering_policy
            .as_ref()
            .map(|policy| policy.policy),
        Some(RoomBufferingPolicy::PauseAnyEligible)
    );
    assert!(extension.prepare.is_none());
    assert_eq!(
        runtime.room_buffering_controls[&room].configured_by_client_id,
        "alice-new"
    );
    assert_eq!(
        runtime.room_buffering_controls[&room].config.policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .requested_config
            .policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
    assert_eq!(runtime.next_playback_barrier_generation, generation);
    assert!(!runtime.room_playback_barriers.contains_key(&room));
}

#[test]
fn policy_subset_retry_cannot_steal_a_combined_start_lifecycle() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old owner should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":95,"requestId":"combined-operation","loadIntent":"newPlayback","logicalMediaId":"combined-media","targetPosition":0.0,"policy":"controller"},"bufferingPolicy":{"mediaGeneration":0,"requestNonce":95,"requestId":"combined-operation","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("combined lifecycle should start");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement transport should overlap");

    let subset = runtime
        .handle_line_fanout_with_transport_actions(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":95,"requestId":"combined-operation","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("policy subset should be rejected without ownership mutation");
    assert!(subset.outbound_lines.is_empty());
    assert!(subset.transport_actions.is_empty());
    assert_eq!(
        runtime.room_playback_barriers["room"].initiator_client_id,
        "alice-old"
    );
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "alice-old"
    );
    assert!(
        !runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );
}

#[test]
fn transport_refresh_tombstones_the_displaced_policy_operation() {
    let room = controlled_room_name_for_test("room", "AB-123-456");
    let mut runtime = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(10.0, 2, 2);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-client", &hello("alice", &room, true))
        .expect("policy owner should join");
    authenticate_policy_controller(&mut runtime, "alice-client");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":96,"requestId":"policy-before-refresh","loadIntent":"newPlayback","policy":"pauseAnyEligible"}}}}"#,
        )
        .expect("initial policy should configure");
    let generation = runtime.next_playback_barrier_generation;
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":97,"requestId":"policy-transport-refresh","loadIntent":"transportRefresh","policy":"pauseAnyEligible"}}}}"#,
        )
        .expect("transport refresh should retain replay protection for the displaced identity");

    assert_eq!(runtime.next_playback_barrier_generation, generation);
    assert_eq!(
        runtime.room_buffering_controls[&room]
            .config
            .request_id
            .as_deref(),
        Some("policy-transport-refresh")
    );
    assert!(runtime.playback_barrier_request_tombstones.contains_key(&(
        room.clone(),
        crate::PlaybackBarrierRequestId::new("policy-before-refresh"),
    )));
    let recovery = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"policy-before-refresh","originalRequestNonce":96,"recoveryNonce":98,"logicalMediaId":"policy-before-refresh-media"}}}}"#,
        )
        .expect("displaced policy recovery should be terminally classified");
    assert_eq!(
        recovery_snapshot_for(&recovery, "alice-client")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Superseded)
    );
}

#[test]
fn stable_request_id_cannot_change_nonce_or_intent_shape() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("participant should join");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":61,"requestId":"stable-policy-id","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("policy-only operation should configure");
    let generation = runtime.next_playback_barrier_generation;

    let changed_nonce = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":62,"requestId":"stable-policy-id","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("same ID with another nonce should be ignored");
    assert!(changed_nonce.is_empty());
    let changed_shape = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":63,"requestId":"stable-policy-id","loadIntent":"newPlayback","logicalMediaId":"shape-change-media","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("policy-only ID must not become a start operation");
    assert!(changed_shape.is_empty());
    assert_eq!(runtime.next_playback_barrier_generation, generation);
    assert!(!runtime.room_playback_barriers.contains_key("room"));
    assert_eq!(
        runtime.room_buffering_controls["room"].config.request_nonce,
        61
    );
}

#[test]
fn generation_after_overlapping_recovery_excludes_the_fenced_transport() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old connection should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":101,"requestId":"fenced-operation-a","loadIntent":"newPlayback","logicalMediaId":"fenced-media-a","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("old connection should establish generation one");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should overlap the old transport");
    runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"fenced-operation-a","originalRequestNonce":101,"recoveryNonce":102,"logicalMediaId":"fenced-media-a"}}}}"#,
        )
        .expect("replacement should recover and fence the old transport");
    assert!(
        runtime
            .playback_barrier_fenced_clients
            .contains("alice-old")
    );

    let replacement = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":103,"requestId":"fenced-operation-b","loadIntent":"newPlayback","logicalMediaId":"fenced-media-b","targetPosition":4.0,"policy":"allEligible"}}}}"#,
        )
        .expect("recovered owner should start a replacement generation");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    let barrier = &runtime.room_playback_barriers["room"];
    assert_eq!(barrier.prepare.media_generation, 2);
    assert_eq!(barrier.participants.len(), 1);
    assert!(barrier.participants.contains_key("alice-new"));
    assert!(!barrier.participants.contains_key("alice-old"));
    let prepare_recipients: BTreeSet<String> = messages(&replacement)
        .into_iter()
        .filter_map(|(recipient, message)| {
            barrier_extension(&message)
                .and_then(|extension| extension.prepare)
                .filter(|prepare| prepare.media_generation == 2)
                .map(|_| recipient)
        })
        .collect();
    assert_eq!(prepare_recipients, BTreeSet::from(["alice-new".to_owned()]));

    let committed = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":2,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("the only eligible replacement participant should commit the barrier");
    assert_eq!(
        playback_barrier_status(&committed).map(|status| status.phase),
        Some(PlaybackBarrierPhase::Committed)
    );
    let stale_ready = runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":2,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("fenced readiness should be safely ignored");
    assert!(stale_ready.is_empty());
}

#[test]
fn invalid_application_request_ids_are_rejected_without_consuming_room_state() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("controller should join");
    let oversized = "r".repeat(crate::PLAYBACK_BARRIER_MAX_REQUEST_ID_BYTES + 1);
    let prepare = serde_json::json!({
        "Set": {
            "sorottePlaybackBarrierV1": {
                "prepare": {
                    "mediaGeneration": 0,
                    "requestNonce": 111,
                    "requestId": oversized,
                    "loadIntent": "newPlayback",
                    "logicalMediaId": "invalid-id-media",
                    "targetPosition": 0.0,
                    "policy": "controller"
                }
            }
        }
    })
    .to_string();
    let rejected_prepare = runtime
        .handle_line_fanout("alice-client", &prepare)
        .expect("oversized request ID should be compatibly rejected");
    assert!(rejected_prepare.is_empty());

    let invalid_policy = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":112,"requestId":"invalid request id","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("non-token request ID should be compatibly rejected");
    assert!(invalid_policy.is_empty());

    let recovery = serde_json::json!({
        "Set": {
            "sorottePlaybackBarrierV1": {
                "recovery": {
                    "requestId": "r".repeat(crate::PLAYBACK_BARRIER_MAX_REQUEST_ID_BYTES + 1),
                    "originalRequestNonce": 111,
                    "recoveryNonce": 113,
                    "logicalMediaId": "invalid-id-media"
                }
            }
        }
    })
    .to_string();
    let rejected_recovery = runtime
        .handle_line_fanout("alice-client", &recovery)
        .expect("invalid recovery identity should receive an explicit rejection");
    assert_eq!(
        recovery_snapshot_for(&rejected_recovery, "alice-client")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Rejected)
    );
    assert_eq!(runtime.next_playback_barrier_generation, 0);
    assert!(runtime.room_playback_barriers.is_empty());
    assert!(runtime.room_buffering_controls.is_empty());
    assert!(runtime.playback_barrier_request_tombstones.is_empty());
}

#[test]
fn retired_request_tombstone_window_starts_at_supersession_and_then_ages_out() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(10.0, 2, 3);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("initial owner should join");
    let operation_a = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"requestId":"aged-operation-a","loadIntent":"newPlayback","logicalMediaId":"aged-media-a","targetPosition":0.0,"policy":"controller"}}}}"#;
    runtime
        .handle_line_fanout("alice-old", operation_a)
        .expect("operation A should start");
    assert!(runtime.playback_barrier_request_tombstones.is_empty());

    // The operation can remain current far longer than the replay window.
    // Its retirement clock must start only when B actually supersedes it.
    runtime.set_playback_barrier_request_clock_for_tests(100.0);
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"requestId":"aged-operation-b","loadIntent":"newPlayback","logicalMediaId":"aged-media-b","targetPosition":1.0,"policy":"controller"}}}}"#,
        )
        .expect("operation B should supersede A");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(
        runtime
            .playback_barrier_request_tombstones
            .get(&(
                "room".to_owned(),
                crate::PlaybackBarrierRequestId::new("aged-operation-a"),
            ))
            .map(|tombstone| tombstone.retain_until_seconds),
        Some(110.0)
    );

    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"ready":{"mediaGeneration":2,"loaded":true,"seekable":true,"bufferReady":true}}}}"#,
        )
        .expect("B should commit");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":2,"stateRevision":1,"observedPosition":1.1}}}}"#,
        )
        .expect("B should complete");
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("a replacement transport should join");

    runtime.set_playback_barrier_request_clock_for_tests(109.0);
    let delayed = runtime
        .handle_line_fanout("alice-new", operation_a)
        .expect("delayed A inside the supported window should be suppressed");
    assert!(delayed.is_empty());
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    let recovery = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"aged-operation-a","originalRequestNonce":1,"recoveryNonce":9,"logicalMediaId":"aged-media-a"}}}}"#,
        )
        .expect("retired A recovery should be explicit");
    assert_eq!(
        recovery_snapshot_for(&recovery, "alice-new")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Superseded)
    );

    runtime.set_playback_barrier_request_clock_for_tests(111.0);
    runtime.prune_playback_barrier_request_tombstones();
    assert!(runtime.playback_barrier_request_tombstones.is_empty());
    runtime
        .handle_line_fanout("alice-new", operation_a)
        .expect("A may become fresh intent after the documented replay window");
    assert_eq!(runtime.next_playback_barrier_generation, 3);
}

#[test]
fn tombstone_capacity_rejection_is_explicit_atomic_and_retryable_after_expiry() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(10.0, 2, 10);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("controller should join");
    for (nonce, request_id, logical_id) in [
        (1, "capacity-a", "capacity-media-a"),
        (2, "capacity-b", "capacity-media-b"),
        (3, "capacity-c", "capacity-media-c"),
    ] {
        runtime
            .handle_line_fanout(
                "alice-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":{nonce},"requestId":"{request_id}","loadIntent":"newPlayback","logicalMediaId":"{logical_id}","targetPosition":0.0,"policy":"controller"}}}}}}}}"#
                ),
            )
            .expect("operation should fit replay capacity");
    }
    assert_eq!(runtime.next_playback_barrier_generation, 3);
    assert_eq!(runtime.playback_barrier_request_tombstones.len(), 2);

    let operation_d = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":4,"requestId":"capacity-d","loadIntent":"newPlayback","logicalMediaId":"capacity-media-d","targetPosition":0.0,"policy":"controller"}}}}"#;
    let rejected = runtime
        .handle_line_fanout("alice-client", operation_d)
        .expect("capacity pressure should return an explicit response");
    let request_result = messages(&rejected)
        .into_iter()
        .find_map(|(_, message)| barrier_extension(&message)?.request_result)
        .expect("capacity pressure should return a typed request result");
    assert_eq!(request_result.request_id, "capacity-d");
    assert_eq!(request_result.request_nonce, 4);
    assert_eq!(
        request_result.status,
        PlaybackBarrierRequestResultStatus::RetryLater
    );
    assert_eq!(request_result.retry_after_ms, 10_000);
    assert_eq!(runtime.next_playback_barrier_generation, 3);
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .request_id
            .as_deref(),
        Some("capacity-c")
    );
    assert_eq!(runtime.playback_barrier_request_nonces["alice-client"], 3);

    runtime.set_playback_barrier_request_clock_for_tests(11.0);
    runtime.prune_playback_barrier_request_tombstones();
    runtime
        .handle_line_fanout("alice-client", operation_d)
        .expect("the same nonce should remain retryable after capacity ages out");
    assert_eq!(runtime.next_playback_barrier_generation, 4);
    assert_eq!(runtime.playback_barrier_request_nonces["alice-client"], 4);
}

#[test]
fn client_application_retries_capacity_pressure_without_stopping_the_session() {
    const CLIENT_ID: &str = "application-client";
    const ROOM: &str = "application-capacity-room";

    let mut server = ServerRuntime::default();
    server.set_playback_barrier_request_tombstone_policy_for_tests(1.0, 1, 10);
    server.set_playback_barrier_request_clock_for_tests(0.0);
    server
        .handle_line(CLIENT_ID, &hello("alice", ROOM, true))
        .expect("application transport should join the server");

    let mut client_session = ClientSession::default();
    client_session
        .apply_message_json(&hello("alice", ROOM, true))
        .expect("application session should negotiate playback coordination");
    let mut application = ClientApplication::new(client_session, DisconnectedPlayer);
    application.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
        policy: Some(sorotte_protocol::PlaybackBarrierPolicy::Controller),
        ..PlaybackBarrierStartConfig::default()
    });

    let send_application_request = |application: &mut ClientApplication<DisconnectedPlayer>,
                                    server: &mut ServerRuntime| {
        let pending = application
            .pending_protocol_line()
            .expect("application request should encode")
            .expect("application request should be queued");
        let line = pending.line().to_owned();
        let ProtocolMessage::Set(set) =
            decode_message_line(&line).expect("application request should decode")
        else {
            panic!("playback coordination should use a Set envelope");
        };
        let prepare = set
            .set
            .playback_barrier_v1()
            .expect("application extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("application request should carry a start prepare");
        application
            .acknowledge_protocol_line(pending.lease())
            .expect("local transport write should release the serialized request");
        let outbound = server
            .handle_line_fanout(CLIENT_ID, &line)
            .expect("server should process the application request");
        (outbound, prepare)
    };

    let apply_application_responses =
        |application: &mut ClientApplication<DisconnectedPlayer>,
         outbound: &[DirectedOutboundLine],
         received_at_seconds: f64| {
            for line in outbound.iter().filter(|line| line.client_id == CLIENT_ID) {
                application
                    .apply_protocol_line(&line.line, received_at_seconds, false, false, false)
                    .expect("application should apply the server response without disconnecting");
            }
        };

    for media_id in ["application-capacity-a", "application-capacity-b"] {
        application.prepare_playback_media(
            LogicalMediaId::new(media_id).expect("test media identity should be valid"),
            MediaTransportKind::NetworkVod,
            0.0,
        );
        let (outbound, _) = send_application_request(&mut application, &mut server);
        apply_application_responses(&mut application, &outbound, 0.0);
    }
    assert_eq!(server.next_playback_barrier_generation, 2);
    assert_eq!(server.playback_barrier_request_tombstones.len(), 1);

    application.prepare_playback_media(
        LogicalMediaId::new("application-capacity-current")
            .expect("current media identity should be valid"),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let (retry_later, initial_prepare) = send_application_request(&mut application, &mut server);
    assert_eq!(server.next_playback_barrier_generation, 2);
    let result = messages(&retry_later)
        .into_iter()
        .find_map(|(client_id, message)| {
            (client_id == CLIENT_ID)
                .then(|| barrier_extension(&message))
                .flatten()
                .and_then(|extension| extension.request_result)
        })
        .expect("capacity pressure should return a correlated retry result");
    assert_eq!(
        result.request_id,
        initial_prepare
            .request_id
            .as_deref()
            .expect("application operation should have a stable identity")
    );
    assert_eq!(result.request_nonce, initial_prepare.request_nonce);
    assert_eq!(
        result.status,
        PlaybackBarrierRequestResultStatus::RetryLater
    );
    assert_eq!(result.retry_after_ms, 1_000);

    apply_application_responses(&mut application, &retry_later, 0.0);
    assert!(
        !application.take_stop_reconnect_requested(),
        "retryable capacity pressure must not terminate reconnect ownership"
    );
    assert!(matches!(
        application.connection_phase(),
        sorotte_client_app::app_boundary::application::ConnectionPhase::Active(_)
    ));
    assert_eq!(
        application.pending_playback_barrier_retry_delay_at(0.0),
        Some(1.0),
        "the semantic current-media intent should remain pending"
    );
    assert_eq!(application.pending_protocol_message_count(), 0);

    application
        .run_pending_playback_barrier_retry_at(0.999)
        .expect("an early retry pump should remain inert");
    assert_eq!(application.pending_protocol_message_count(), 0);
    application
        .run_pending_playback_barrier_retry_at(1.1)
        .expect("the due retry should be queued");
    application
        .run_pending_playback_barrier_retry_at(1.1)
        .expect("a repeated retry pump should be idempotent");
    assert_eq!(
        application.pending_protocol_message_count(),
        1,
        "exactly one retry should be serialized"
    );

    server.set_playback_barrier_request_clock_for_tests(1.1);
    let (accepted, retried_prepare) = send_application_request(&mut application, &mut server);
    assert_eq!(
        retried_prepare.request_id, initial_prepare.request_id,
        "retry must retain the same operation identity"
    );
    assert_eq!(
        retried_prepare.request_nonce, initial_prepare.request_nonce,
        "retry must retain the same operation nonce"
    );
    assert_eq!(server.next_playback_barrier_generation, 3);
    assert_eq!(
        server.room_playback_barriers[ROOM].prepare.request_id,
        initial_prepare.request_id
    );
    apply_application_responses(&mut application, &accepted, 1.1);
    assert_eq!(
        application.pending_playback_barrier_retry_delay_at(1.1),
        None
    );
    assert_eq!(application.pending_protocol_message_count(), 0);
    assert!(!application.take_stop_reconnect_requested());
}

#[test]
fn new_identity_rate_limit_is_correlated_and_exact_retries_and_recovery_are_exempt() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_new_identity_rate_policy_for_tests(2.0, 1, 1);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("controller should join");

    let operation_a = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"requestId":"rate-a","loadIntent":"newPlayback","logicalMediaId":"rate-media-a","targetPosition":0.0,"policy":"controller"}}}}"#;
    runtime
        .handle_line_fanout("alice-client", operation_a)
        .expect("first identity should fit the burst budget");
    assert_eq!(runtime.next_playback_barrier_generation, 1);

    let operation_b = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"requestId":"rate-b","loadIntent":"newPlayback","logicalMediaId":"rate-media-b","targetPosition":0.0,"policy":"controller"}}}}"#;
    for _ in 0..2 {
        let rejected = runtime
            .handle_line_fanout("alice-client", operation_b)
            .expect("rate pressure should return a typed retry result");
        let result = messages(&rejected)
            .into_iter()
            .find_map(|(_, message)| barrier_extension(&message)?.request_result)
            .expect("rate rejection should be correlated");
        assert_eq!(result.request_id, "rate-b");
        assert_eq!(result.request_nonce, 2);
        assert_eq!(
            result.status,
            PlaybackBarrierRequestResultStatus::RetryLater
        );
        assert_eq!(result.retry_after_ms, 2_000);
    }
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert_eq!(runtime.playback_barrier_request_nonces["alice-client"], 1);

    let recovery = runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"rate-a","originalRequestNonce":1,"recoveryNonce":9,"logicalMediaId":"rate-media-a"}}}}"#,
        )
        .expect("recovery query should not share the new-identity budget");
    let recovered = recovery_snapshot_for(&recovery, "alice-client")
        .and_then(|extension| extension.recovery)
        .expect("current operation should remain recoverable while rate-limited");
    assert_eq!(
        recovered.disposition,
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );

    runtime.set_playback_barrier_request_clock_for_tests(2.1);
    runtime
        .handle_line_fanout("alice-client", operation_b)
        .expect("the exact rejected operation should succeed after the retry delay");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(runtime.playback_barrier_request_nonces["alice-client"], 2);

    runtime
        .handle_line_fanout("alice-client", operation_b)
        .expect("an exact accepted retry should bypass new-identity rate accounting");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
}

#[test]
fn capacity_retry_keeps_its_room_rate_exemption_across_reconnect() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(10.0, 1, 10);
    runtime.set_playback_barrier_new_identity_rate_policy_for_tests(10.0, 10, 4);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("filler-client", &hello("filler", "room", true))
        .expect("filler should join");
    for (nonce, request_id, logical_id) in [
        (1, "reconnect-rate-a", "reconnect-rate-media-a"),
        (2, "reconnect-rate-b", "reconnect-rate-media-b"),
    ] {
        runtime
            .handle_line_fanout(
                "filler-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":{nonce},"requestId":"{request_id}","loadIntent":"newPlayback","logicalMediaId":"{logical_id}","targetPosition":0.0,"policy":"controller"}}}}}}}}"#
                ),
            )
            .expect("filler operation should be accepted");
    }
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(runtime.playback_barrier_request_tombstones.len(), 1);

    let retry = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":1,"requestId":"capacity-reconnect-retry","loadIntent":"newPlayback","logicalMediaId":"capacity-reconnect-media","targetPosition":0.0,"policy":"controller"}}}}"#;
    runtime
        .handle_line("owner-old", &hello("owner", "room", true))
        .expect("original owner transport should join");
    let first_rejection = runtime
        .handle_line_fanout("owner-old", retry)
        .expect("capacity pressure should reject the original transport nonfatally");
    assert!(messages(&first_rejection).into_iter().any(|(_, message)| {
        barrier_extension(&message)
            .and_then(|extension| extension.request_result)
            .is_some_and(|result| result.request_id == "capacity-reconnect-retry")
    }));
    assert_eq!(
        runtime.playback_barrier_new_identity_rate_by_room["room"].len(),
        3
    );

    runtime
        .handle_transport_disconnect_fanout("owner-old")
        .expect("old owner transport should disconnect");
    runtime
        .handle_line("owner-new", &hello("owner", "room", true))
        .expect("replacement owner transport should join");
    let reconnect_rejection = runtime
        .handle_line_fanout("owner-new", retry)
        .expect("the same operation should remain retryable after reconnect");
    let result = messages(&reconnect_rejection)
        .into_iter()
        .find_map(|(_, message)| barrier_extension(&message)?.request_result)
        .expect("reconnect should still reach replay-capacity admission");
    assert_eq!(result.request_id, "capacity-reconnect-retry");
    assert_eq!(result.request_nonce, 1);
    assert_eq!(result.retry_after_ms, 10_000);
    assert_eq!(
        runtime.playback_barrier_new_identity_rate_by_room["room"].len(),
        3,
        "one logical operation must not consume another room-rate slot after reconnect"
    );
    assert_eq!(runtime.next_playback_barrier_generation, 2);
}

#[test]
fn no_id_operation_is_charged_again_after_reconnect() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_new_identity_rate_policy_for_tests(10.0, 10, 1);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("keeper", &hello("keeper", "room", true))
        .expect("keeper should retain the room across reconnect");
    runtime
        .handle_line("owner-old", &hello("owner", "room", true))
        .expect("old owner should join");
    let policy = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":1,"loadIntent":"newPlayback","policy":"independent"}}}}"#;
    runtime
        .handle_line_fanout("owner-old", policy)
        .expect("first no-ID operation should fit the room budget");
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id,
        "owner-old"
    );

    runtime
        .handle_transport_disconnect_fanout("owner-old")
        .expect("old owner should disconnect");
    runtime
        .handle_line("owner-new", &hello("owner", "room", true))
        .expect("new owner transport should join");
    let rejected = runtime
        .handle_line_fanout("owner-new", policy)
        .expect("uncorrelated operation should be rejected without a fatal error");
    assert!(rejected.is_empty());
    assert_eq!(
        runtime.room_buffering_controls["room"].configured_by_client_id, "owner-old",
        "missing operation IDs must not gain an exact-retry exemption across transports"
    );
    assert_eq!(
        runtime.playback_barrier_new_identity_rate_by_room["room"].len(),
        1
    );
}

#[test]
fn replacement_hello_does_not_reset_the_connection_identity_budget() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_new_identity_rate_policy_for_tests(10.0, 1, 10);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("keeper", &hello("keeper", "room", true))
        .expect("keeper should retain the room");
    runtime
        .handle_line("owner-client", &hello("owner", "room", true))
        .expect("owner should join");
    runtime
        .handle_line_fanout(
            "owner-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":1,"requestId":"hello-rate-a","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("first identity should consume the connection budget");

    runtime
        .handle_line("owner-client", &hello("owner", "room", true))
        .expect("replacement Hello should keep the physical connection budget");
    let rejected = runtime
        .handle_line_fanout(
            "owner-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":2,"requestId":"hello-rate-b","loadIntent":"newPlayback","policy":"independent"}}}}"#,
        )
        .expect("second identity should receive typed backpressure");
    let result = messages(&rejected)
        .into_iter()
        .find_map(|(_, message)| barrier_extension(&message)?.request_result)
        .expect("replacement Hello must not bypass the per-connection rate limit");
    assert_eq!(result.request_id, "hello-rate-b");
    assert_eq!(result.request_nonce, 2);
    assert_eq!(
        result.status,
        PlaybackBarrierRequestResultStatus::RetryLater
    );
    assert_eq!(
        runtime.room_buffering_controls["room"]
            .config
            .request_id
            .as_deref(),
        Some("hello-rate-a")
    );
}

#[test]
fn room_new_identity_rate_limit_covers_policy_only_churn_across_connections() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_new_identity_rate_policy_for_tests(1.0, 10, 1);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-client", &hello("alice", "public-room", true))
        .expect("first capable participant should join");
    runtime
        .handle_line("bob-client", &hello("bob", "public-room", true))
        .expect("second capable participant should join");

    let policy_a = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":1,"requestId":"room-rate-a","loadIntent":"newPlayback","policy":"independent"}}}}"#;
    runtime
        .handle_line_fanout("alice-client", policy_a)
        .expect("first room identity should fit the room budget");
    assert_eq!(runtime.next_playback_barrier_generation, 1);

    let policy_b = r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{"mediaGeneration":0,"requestNonce":1,"requestId":"room-rate-b","loadIntent":"newPlayback","policy":"independent"}}}}"#;
    let rejected = runtime
        .handle_line_fanout("bob-client", policy_b)
        .expect("room pressure should produce a typed result");
    let result = messages(&rejected)
        .into_iter()
        .find_map(|(_, message)| barrier_extension(&message)?.request_result)
        .expect("room rate rejection should be correlated");
    assert_eq!(result.request_id, "room-rate-b");
    assert_eq!(result.request_nonce, 1);
    assert_eq!(
        result.status,
        PlaybackBarrierRequestResultStatus::RetryLater
    );
    assert_eq!(result.retry_after_ms, 1_000);
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert!(
        !runtime
            .playback_barrier_request_nonces
            .contains_key("bob-client")
    );

    runtime.set_playback_barrier_request_clock_for_tests(1.1);
    runtime
        .handle_line_fanout("bob-client", policy_b)
        .expect("policy-only operation should retry after the room window");
    // Equivalent public Independent policy is deliberately coalesced rather
    // than allocating a fresh room generation.
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert_eq!(runtime.playback_barrier_request_nonces["bob-client"], 1);
    assert_eq!(
        runtime.room_buffering_controls["public-room"]
            .config
            .request_id
            .as_deref(),
        Some("room-rate-b")
    );
}

#[test]
fn tombstone_global_bound_ages_across_retained_persistent_rooms() {
    let mut runtime = ServerRuntime::default();
    runtime.set_persistent_rooms_enabled(true);
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(5.0, 4, 2);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    for room_index in 1..=3 {
        let client_id = format!("client-{room_index}");
        let room = format!("room-{room_index}");
        runtime
            .handle_line(&client_id, &hello("alice", &room, true))
            .expect("room owner should join");
        runtime
            .handle_line_fanout(
                &client_id,
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":1,"requestId":"global-{room_index}-a","loadIntent":"newPlayback","logicalMediaId":"global-media-{room_index}-a","targetPosition":0.0,"policy":"controller"}}}}}}}}"#
                ),
            )
            .expect("first room operation should start");
        if room_index < 3 {
            runtime
                .handle_line_fanout(
                    &client_id,
                    &format!(
                        r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":2,"requestId":"global-{room_index}-b","loadIntent":"newPlayback","logicalMediaId":"global-media-{room_index}-b","targetPosition":0.0,"policy":"controller"}}}}}}}}"#
                    ),
                )
                .expect("replacement should create one tombstone");
            runtime
                .handle_line_fanout(
                    &client_id,
                    r#"{"Set":{"playlistChange":{"files":["retained.mkv"]}}}"#,
                )
                .expect("room should gain retained persistent state");
            runtime
                .handle_transport_disconnect_fanout(&client_id)
                .expect("retained room owner should disconnect");
            assert!(runtime.clients_in_room(&room).is_empty());
            assert!(runtime.room_should_be_retained_when_empty(&room));
        }
    }
    assert_eq!(runtime.playback_barrier_request_tombstones.len(), 2);
    let third_room_replacement = r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":2,"requestId":"global-3-b","loadIntent":"newPlayback","logicalMediaId":"global-media-3-b","targetPosition":0.0,"policy":"controller"}}}}"#;
    let rejected = runtime
        .handle_line_fanout("client-3", third_room_replacement)
        .expect("global capacity should reject without mutation");
    let request_result = messages(&rejected)
        .into_iter()
        .find_map(|(_, message)| barrier_extension(&message)?.request_result)
        .expect("global capacity should return a correlated retry result");
    assert_eq!(request_result.request_id, "global-3-b");
    assert_eq!(request_result.request_nonce, 2);
    assert_eq!(
        request_result.status,
        PlaybackBarrierRequestResultStatus::RetryLater
    );
    assert_eq!(request_result.retry_after_ms, 5_000);
    assert_eq!(runtime.next_playback_barrier_generation, 5);

    runtime.set_playback_barrier_request_clock_for_tests(6.0);
    runtime
        .collect_dispatch_at(0.0)
        .expect("periodic maintenance should prune expired tombstones globally");
    assert!(runtime.playback_barrier_request_tombstones.is_empty());
    runtime
        .handle_line_fanout("client-3", third_room_replacement)
        .expect("global capacity should recover after bounded aging");
    assert_eq!(runtime.next_playback_barrier_generation, 6);
}

#[test]
fn public_independent_policy_churn_is_coalesced_without_tombstones() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(10.0, 1, 1);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("public participant should join");
    for index in 1..=8 {
        let load_intent = if index % 2 == 0 {
            "transportRefresh"
        } else {
            "newPlayback"
        };
        let response = runtime
            .handle_line_fanout(
                "alice-client",
                &format!(
                    r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"bufferingPolicy":{{"mediaGeneration":0,"requestNonce":{index},"requestId":"public-independent-{index}","loadIntent":"{load_intent}","policy":"independent"}}}}}}}}"#
                ),
            )
            .expect("harmless Independent request should receive a canonical ack");
        assert!(buffering_snapshot_for(&response, "alice-client").is_some());
    }
    assert_eq!(runtime.next_playback_barrier_generation, 1);
    assert!(runtime.playback_barrier_request_tombstones.is_empty());
    assert_eq!(
        runtime.room_buffering_controls["room"]
            .config
            .request_id
            .as_deref(),
        Some("public-independent-8")
    );

    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":20,"requestId":"legitimate-start","loadIntent":"newPlayback","logicalMediaId":"legitimate-media","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("legitimate start should remain available immediately");
    assert_eq!(runtime.next_playback_barrier_generation, 2);
    assert_eq!(runtime.playback_barrier_request_tombstones.len(), 1);
}

#[test]
fn current_operation_remains_recoverable_after_tombstone_ttl() {
    let mut runtime = ServerRuntime::default();
    runtime.set_playback_barrier_request_tombstone_policy_for_tests(1.0, 1, 1);
    runtime.set_playback_barrier_request_clock_for_tests(0.0);
    runtime
        .handle_line("alice-old", &hello("alice", "room", true))
        .expect("old owner should join");
    runtime
        .handle_line_fanout(
            "alice-old",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":31,"requestId":"current-aged-operation","loadIntent":"newPlayback","logicalMediaId":"current-aged-media","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("current operation should start");
    runtime.set_playback_barrier_request_clock_for_tests(100.0);
    runtime.prune_playback_barrier_request_tombstones();
    assert!(runtime.playback_barrier_request_tombstones.is_empty());
    // Even if an imported/older runtime state redundantly retained the same
    // identity as a tombstone, canonical current state must win.
    runtime.playback_barrier_request_tombstones.insert(
        (
            "room".to_owned(),
            crate::PlaybackBarrierRequestId::new("current-aged-operation"),
        ),
        crate::PlaybackBarrierRequestTombstone {
            request_nonce: 31,
            logical_media_id_digest: None,
            media_generation: 1,
            retain_until_seconds: 200.0,
        },
    );
    runtime
        .handle_line("alice-new", &hello("alice", "room", true))
        .expect("replacement should join");
    let recovered = runtime
        .handle_line_fanout(
            "alice-new",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"recovery":{"requestId":"current-aged-operation","originalRequestNonce":31,"recoveryNonce":32,"logicalMediaId":"current-aged-media"}}}}"#,
        )
        .expect("current identity should recover from canonical room state");
    assert_eq!(
        recovery_snapshot_for(&recovered, "alice-new")
            .and_then(|extension| extension.recovery)
            .and_then(|recovery| recovery.disposition),
        Some(PlaybackBarrierRecoveryDisposition::Recovered)
    );
    assert_eq!(runtime.next_playback_barrier_generation, 1);
}

#[test]
fn playback_request_identity_is_redacted_from_server_runtime_debug() {
    const REQUEST_MARKER: &str = "debug-request-secret-canary";
    const LOGICAL_MEDIA_MARKER: &str = "debug-logical-secret-canary";
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("alice-client", &hello("alice", "room", true))
        .expect("controller should join");
    runtime
        .handle_line_fanout(
            "alice-client",
            &format!(
                r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"prepare":{{"mediaGeneration":0,"requestNonce":121,"requestId":"{REQUEST_MARKER}","loadIntent":"newPlayback","logicalMediaId":"{LOGICAL_MEDIA_MARKER}","targetPosition":0.0,"policy":"controller"}},"bufferingPolicy":{{"mediaGeneration":0,"requestNonce":121,"requestId":"{REQUEST_MARKER}","loadIntent":"newPlayback","policy":"independent"}}}}}}}}"#
            ),
        )
        .expect("request identity should be retained only in redacted carriers");
    runtime
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"sorottePlaybackBarrierV1":{"prepare":{"mediaGeneration":0,"requestNonce":122,"requestId":"debug-replacement-operation","loadIntent":"newPlayback","logicalMediaId":"debug-replacement-media","targetPosition":0.0,"policy":"controller"}}}}"#,
        )
        .expect("replacement should retire the canary identity into the bounded cache");

    let debug = format!("{runtime:?}");
    assert!(!debug.contains(REQUEST_MARKER));
    assert!(!debug.contains(LOGICAL_MEDIA_MARKER));
    assert!(debug.contains("<redacted-playback-request-id>"));
    assert!(debug.contains("<redacted>"));
}
