use super::*;
use sorotte_client_app::app_boundary::readiness::ParticipantReadinessPresentation;
use sorotte_client_core::{
    ClientRuntime, ClientSession, LogicalMediaId, MediaTransportKind, QueuedRuntimeControl,
};
use sorotte_player_api::{
    DisconnectedPlayer, PlayerMediaGeneration, PlayerObservationTimestamp, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    DirectReadinessSurface, MediaReadyPayload, PlaybackBarrierPhase, PlaybackBarrierStateExtension,
    ReadinessIntentRequest, ReadinessSetExtension, ReadinessStateExtension, StatePayload,
    TechnicalPlayability, TechnicalPlayabilityPhase, TechnicalReadinessReport, UserReadinessIntent,
    UserReadinessMutationSource, encode_message_line,
};

fn exchange(runtime: &mut ServerRuntime, client: &str, line: &str) -> Vec<DirectedOutboundLine> {
    let replies = runtime.handle_line_fanout(client, line).unwrap();
    acknowledge_directed_state_counters(runtime, &decode_directed_lines(&replies));
    replies
}

fn join(
    runtime: &mut ServerRuntime,
    client: &str,
    room: &str,
    v2: bool,
) -> Vec<DirectedOutboundLine> {
    exchange(
        runtime,
        client,
        &json!({"Hello": {
            "username": client, "room": {"name": room}, "version": "1.7.5",
            "features": {"sorotteReadinessV2": v2, "sorottePlaybackBarrierV1": true}
        }})
        .to_string(),
    )
}

fn intent(runtime: &mut ServerRuntime, client: &str, nonce: u64, desired: UserReadinessIntent) {
    let session = &runtime.sessions[client];
    let epoch = runtime.room_readiness[&session.room].participants[&session.username]
        .record
        .membership_epoch;
    let request = ReadinessIntentRequest::new(
        format!("{client}-intent-{nonce}"),
        nonce,
        epoch,
        desired,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    );
    let message = ProtocolMessage::set(
        SetPayload::new().with_readiness_v2(ReadinessSetExtension::new().with_intent(request)),
    );
    exchange(runtime, client, &encode_message_line(&message).unwrap());
}

fn prepare(runtime: &mut ServerRuntime, policy: &str, nonce: u64) -> Vec<DirectedOutboundLine> {
    exchange(
        runtime,
        "alice",
        &json!({"Set": {"sorottePlaybackBarrierV1": {"prepare": {
            "mediaGeneration": 0, "requestNonce": nonce, "requestId": format!("membership-start-{nonce}"),
            "loadIntent": if nonce == 1 { "newPlayback" } else { "replay" },
            "logicalMediaId": "membership:movie", "targetPosition": 0.0,
            "policy": policy, "quorumPercent": 100, "timeoutMs": 30000,
            "timeoutAction": "askController"
        }, "bufferingPolicy": {
            "mediaGeneration": 0, "requestNonce": nonce, "requestId": format!("membership-start-{nonce}"),
            "loadIntent": if nonce == 1 { "newPlayback" } else { "replay" },
            "policy": "independent"
        }}}})
        .to_string(),
    )
}

fn playable(
    runtime: &mut ServerRuntime,
    client: &str,
    generation: u64,
) -> Vec<DirectedOutboundLine> {
    let session = &runtime.sessions[client];
    let record = &runtime.room_readiness[&session.room].participants[&session.username].record;
    let technical = TechnicalReadinessReport::new(
        generation,
        record.membership_epoch,
        record.last_technical_report_sequence + 1,
        TechnicalPlayabilityPhase::Playable,
    );
    let message = ProtocolMessage::state(
        StatePayload::new()
            .with_readiness_v2(ReadinessStateExtension::new().with_technical(technical))
            .with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new()
                    .with_ready(MediaReadyPayload::new(generation, true, true)),
            ),
    );
    exchange(runtime, client, &encode_message_line(&message).unwrap())
}

type ProbeClient = ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>;

fn deliver_bob(client: &mut ProbeClient, lines: &[DirectedOutboundLine], now: f64) {
    for (recipient, message) in decode_directed_lines(lines) {
        if recipient == "bob" {
            // ClientApplication uses this State-versus-other dispatch with
            // reconciliation enabled in a connected GUI/CLI session.
            match message {
                ProtocolMessage::State(state) => {
                    client.run_state_sync_reconcile_with_inbound_state_with_ping_at_clocks(
                        state.state,
                        false,
                        now,
                        now,
                        now,
                    );
                }
                other => client
                    .session_mut()
                    .apply_protocol_message_at(other, now)
                    .unwrap(),
            }
        }
    }
}

fn flush_bob(
    server: &mut ServerRuntime,
    client: &mut ProbeClient,
    now: f64,
) -> Vec<TechnicalReadinessReport> {
    let mut reports = Vec::new();
    let mut count = 0;
    while let Some(pending) = client.pending_protocol_line().unwrap() {
        count += 1;
        assert!(count < 100, "ordinary response exchange must finish");
        let line = pending.line().to_owned();
        client.acknowledge_protocol_line(pending.lease()).unwrap();
        if let ProtocolMessage::State(state) = decode_message_line(&line).unwrap()
            && let Some(extension) = state.state.readiness_v2().unwrap()
            && let Some(report) = extension.technical
        {
            println!("Bob generated technical report: {report:?}");
            reports.push(report);
        }
        let replies = exchange(server, "bob", &line);
        deliver_bob(client, &replies, now);
    }
    reports
}

fn bob_playing(second: u64) -> PlayerTransportTelemetryUpdate {
    let mut update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(second)),
    )
    .with_phase(PlayerTransportPhase::Playing)
    .with_position_seconds(second as f64)
    .with_logical_pause(false);
    update.paused_for_cache = Some(false);
    update.seeking = Some(false);
    update.seekable = Some(true);
    update.buffered_ahead_seconds = Some(10.0);
    update
}

fn late_join_technical_readiness(
    join_after_commit: bool,
    complete_before_join: bool,
) -> (TechnicalPlayability, Vec<TechnicalReadinessReport>) {
    let mut server = ServerRuntime::default();
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    join(&mut server, "alice", "room", true);
    intent(&mut server, "alice", 1, UserReadinessIntent::Ready);
    let mut bob = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    if !join_after_commit {
        let hello = join(&mut server, "bob", "room", true);
        deliver_bob(&mut bob, &hello, 100.0);
        assert!(bob.run_toggle_ready(true).unwrap());
        flush_bob(&mut server, &mut bob, 100.0);
    }
    let prepared = prepare(&mut server, "allEligible", 1);
    if !join_after_commit {
        deliver_bob(&mut bob, &prepared, 100.0);
    }
    let alice_ready = playable(&mut server, "alice", 1);
    if !join_after_commit {
        deliver_bob(&mut bob, &alice_ready, 100.0);
        let bob_ready = playable(&mut server, "bob", 1);
        deliver_bob(&mut bob, &bob_ready, 100.0);
    }
    assert_eq!(
        server.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Committed
    );
    assert_eq!(
        server.room_playback_barriers["room"].state_revision,
        Some(1)
    );
    if complete_before_join || !join_after_commit {
        let started = exchange(
            &mut server,
            "alice",
            r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":0.1}}}}"#,
        );
        if !join_after_commit {
            deliver_bob(&mut bob, &started, 100.0);
            let started = exchange(
                &mut server,
                "bob",
                r#"{"State":{"sorottePlaybackBarrierV1":{"started":{"mediaGeneration":1,"stateRevision":1,"observedPosition":0.1}}}}"#,
            );
            deliver_bob(&mut bob, &started, 100.0);
        }
        assert_eq!(
            server.room_playback_barriers["room"].phase,
            PlaybackBarrierPhase::Complete
        );
    }
    if join_after_commit {
        let hello = join(&mut server, "bob", "room", true);
        deliver_bob(&mut bob, &hello, 100.0);
        assert!(bob.run_toggle_ready(true).unwrap());
        flush_bob(&mut server, &mut bob, 100.0);
    }
    println!(
        "join_after_commit={join_after_commit}, complete_before_join={complete_before_join}, Bob retained commit={:?}",
        bob.session().playback_barrier_commit()
    );
    bob.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("membership:movie").unwrap(),
        MediaTransportKind::LocalFile,
        100.0,
    );
    let mut reports = Vec::new();
    for second in 1..=15 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        bob.observe_external_player_transport(bob_playing(second), now);
        reports.extend(flush_bob(&mut server, &mut bob, now));
        if second == 11 {
            // Pass the original start acknowledgement deadline using the
            // public periodic server path; the late join remains playable
            // after that deadline without another coordination request.
            let dispatch = server.collect_dispatch_at(now).unwrap();
            acknowledge_directed_state_counters(
                &mut server,
                &decode_directed_lines(&dispatch.outbound_lines),
            );
            deliver_bob(&mut bob, &dispatch.outbound_lines, now);
            reports.extend(flush_bob(&mut server, &mut bob, now));
        }
    }
    let observed = server.room_readiness["room"].participants["bob"]
        .record
        .technical_state
        .clone();
    println!("Bob server technical state after fifteen fresh playing samples: {observed:?}");
    let canonical = bob
        .session()
        .canonical_participant_readiness("bob")
        .unwrap();
    let presentation = ParticipantReadinessPresentation::from_v2(canonical, None);
    println!(
        "Bob public wire state: phase={:?}, room_ready={}, last_sequence={}, UI status={:?}",
        canonical.technical_state.phase,
        canonical.room_ready,
        canonical.last_technical_report_sequence,
        presentation.status_label()
    );
    assert_eq!(canonical.user_intent, UserReadinessIntent::Ready);
    assert_eq!(
        canonical.technical_state.phase,
        if matches!(observed, TechnicalPlayability::Playable { .. }) {
            TechnicalPlayabilityPhase::Playable
        } else {
            TechnicalPlayabilityPhase::Preparing
        }
    );
    assert_eq!(
        presentation.technical_status_suffix(),
        (!matches!(observed, TechnicalPlayability::Playable { .. })).then_some("loading")
    );
    assert!(bob.run_toggle_ready(true).unwrap());
    flush_bob(&mut server, &mut bob, 116.0);
    assert_eq!(
        server.room_readiness["room"].participants["bob"]
            .record
            .user_intent,
        UserReadinessIntent::NotReady,
        "the next normal explicit Ready toggle must remain accepted"
    );
    (observed, reports)
}

#[test]
fn joining_committed_playback_receives_the_technical_revision() {
    let (observed, reports) = late_join_technical_readiness(true, false);
    assert!(
        reports
            .iter()
            .any(|report| report.phase == TechnicalPlayabilityPhase::Playable)
    );
    assert!(
        matches!(observed, TechnicalPlayability::Playable { .. }),
        "a newly joined member with fresh playing telemetry should become technically playable"
    );
}

#[test]
fn joining_preparing_playback_receives_the_technical_revision() {
    let (observed, reports) = late_join_technical_readiness(false, false);
    assert!(
        reports
            .iter()
            .any(|report| report.phase == TechnicalPlayabilityPhase::Playable)
    );
    assert!(matches!(observed, TechnicalPlayability::Playable { .. }));
}

#[test]
fn joining_completed_playback_receives_the_technical_revision() {
    let (observed, reports) = late_join_technical_readiness(true, true);
    assert!(
        reports
            .iter()
            .any(|report| report.phase == TechnicalPlayabilityPhase::Playable)
    );
    assert!(
        matches!(observed, TechnicalPlayability::Playable { .. }),
        "a member joining completed coordinated start should not remain loading while playing"
    );
}
