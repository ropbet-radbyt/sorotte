use super::*;
use sorotte_client_core::{
    ClientRuntime, ClientSession, LogicalMediaId, MediaTransportKind, QueuedRuntimeControl,
};
use sorotte_player_api::{
    DisconnectedPlayer, PlayerMediaGeneration, PlayerObservationTimestamp, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    RoomPauseOwner, RoomStartGatePhase, TechnicalPlayability, UserReadinessIntent,
};

type ProbeClient = ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>;

fn hello(username: &str, room: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"sorottePlaybackBarrierV1":true}}}}}}"#
    )
}

fn deliver_to_bob(client: &mut ProbeClient, lines: &[DirectedOutboundLine], now: f64) {
    deliver_to_client("bob-client", client, lines, now);
}

fn deliver_to_client(
    client_id: &str,
    client: &mut ProbeClient,
    lines: &[DirectedOutboundLine],
    now: f64,
) {
    for (recipient, message) in decode_directed_lines(lines) {
        if recipient == client_id {
            // Same State-vs-other dispatch as ClientApplication::apply_protocol_line
            // with reconciliation enabled in GUI/CLI connected sessions.
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

fn flush_bob(server: &mut ServerRuntime, client: &mut ProbeClient, now: f64) -> usize {
    let mut reports = 0;
    while let Some(pending) = client.pending_protocol_line().unwrap() {
        let line = pending.line().to_owned();
        client.acknowledge_protocol_line(pending.lease()).unwrap();
        let decoded = decode_message_line(&line).unwrap();
        if matches!(&decoded, ProtocolMessage::State(state)
            if state.state.playback_barrier_v1().unwrap().is_some_and(|ext| ext.transport.is_some()))
        {
            reports += 1;
        }
        let replies = server.handle_line_fanout("bob-client", &line).unwrap();
        deliver_to_bob(client, &replies, now);
    }
    reports
}

fn transport(second: u64, buffering: bool) -> PlayerTransportTelemetryUpdate {
    let mut update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(second)),
    )
    .with_phase(if buffering {
        PlayerTransportPhase::Rebuffering
    } else {
        PlayerTransportPhase::Playing
    })
    .with_position_seconds(10.0)
    .with_logical_pause(false);
    update.paused_for_cache = Some(buffering);
    update.seeking = Some(false);
    update.seekable = Some(true);
    update.buffered_ahead_seconds = Some(if buffering { 0.0 } else { 10.0 });
    update
}

fn buffering_fixture() -> (ServerRuntime, ProbeClient, String) {
    buffering_fixture_with_timing(0, 0)
}

fn buffering_fixture_with_timing(
    debounce_ms: u64,
    resume_hysteresis_ms: u64,
) -> (ServerRuntime, ProbeClient, String) {
    let room = controlled_room_name_for_test("buffering-room", "AB-123-456");
    let mut server = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    server
        .handle_line("alice-client", &hello("alice", &room))
        .unwrap();
    let bob_hello = server
        .handle_line_fanout("bob-client", &hello("bob", &room))
        .unwrap();
    server
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .unwrap();
    server
        .handle_line_fanout(
            "alice-client",
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false}}}"#,
        )
        .unwrap();
    assert!(!server.room_playback_state(&room).paused);
    let policy = server
        .handle_line_fanout(
            "alice-client",
            &format!(r#"{{"Set":{{"sorottePlaybackBarrierV1":{{"bufferingPolicy":{{"mediaGeneration":0,"requestNonce":1,"requestId":"bughunt-buffering","loadIntent":"newPlayback","policy":"pauseAnyEligible","debounceMs":{debounce_ms},"resumeHysteresisMs":{resume_hysteresis_ms},"maxPauseMs":30000}}}}}}}}"#),
        )
        .unwrap();
    let mut bob = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    deliver_to_bob(&mut bob, &bob_hello, 100.0);
    bob.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("bughunt-media").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    deliver_to_bob(&mut bob, &policy, 100.0);
    flush_bob(&mut server, &mut bob, 100.0);
    (server, bob, room)
}

#[test]
fn fresh_ongoing_buffering_must_not_expire_before_maximum_pause() {
    let (mut server, mut bob, room) = buffering_fixture();
    let mut reports = Vec::new();
    for second in 1..=5 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        let mut observation = transport(second, true);
        observation.logical_pause = Some(server.room_playback_state(&room).paused);
        bob.observe_external_player_transport(observation, now);
        reports.push(flush_bob(&mut server, &mut bob, now));
        let tick = server.collect_dispatch_at(now).unwrap();
        deliver_to_bob(&mut bob, &tick.outbound_lines, now);
        if second == 1 {
            assert!(server.room_playback_state(&room).paused);
        }
    }
    assert!(server.session("bob-client").is_some());
    assert!(
        server.room_playback_state(&room).paused,
        "fresh cache-stall evidence arrived every second and the 30-second maximum pause has not elapsed; report counts were {reports:?}"
    );
}

#[test]
fn default_buffering_timers_keep_fresh_stall_paused() {
    let (mut server, mut bob, room) = buffering_fixture_with_timing(750, 1500);
    let mut reports = Vec::new();
    for second in 1..=8 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        let mut observation = transport(second, true);
        observation.logical_pause = Some(server.room_playback_state(&room).paused);
        bob.observe_external_player_transport(observation, now);
        reports.push(flush_bob(&mut server, &mut bob, now));
        let tick = server.collect_dispatch_at(now).unwrap();
        deliver_to_bob(&mut bob, &tick.outbound_lines, now);
        if second == 2 {
            assert!(server.room_playback_state(&room).paused);
        }
    }
    assert!(
        server.room_playback_state(&room).paused,
        "default debounce/hysteresis must retain the pause for a freshly observed ongoing stall before its 30-second bound; report counts were {reports:?}"
    );
}

#[test]
fn recovered_buffering_client_releases_pause_control() {
    let (mut server, mut bob, room) = buffering_fixture();
    server.set_clock_overrides_seconds(Some(101.0), Some(1.0));
    bob.observe_external_player_transport(transport(1, true), 101.0);
    assert_eq!(flush_bob(&mut server, &mut bob, 101.0), 1);
    assert!(server.room_playback_state(&room).paused);
    server.set_clock_overrides_seconds(Some(102.0), Some(2.0));
    bob.observe_external_player_transport(transport(2, false), 102.0);
    assert_eq!(flush_bob(&mut server, &mut bob, 102.0), 1);
    assert!(!server.room_playback_state(&room).paused);
}

#[test]
fn duplicate_buffering_samples_and_reconciliation_do_not_extend_stale_evidence() {
    let (mut server, mut bob, room) = buffering_fixture();
    server.set_clock_overrides_seconds(Some(101.0), Some(1.0));
    bob.observe_external_player_transport(transport(1, true), 101.0);
    assert_eq!(flush_bob(&mut server, &mut bob, 101.0), 1);
    assert!(server.room_playback_state(&room).paused);
    for second in 2..=8 {
        let now = 100.0 + f64::from(second);
        server.set_clock_overrides_seconds(Some(now), Some(f64::from(second)));
        bob.observe_external_player_transport(transport(1, true), now);
        assert_eq!(flush_bob(&mut server, &mut bob, now), 0);
        let tick = server.collect_dispatch_at(now).unwrap();
        deliver_to_bob(&mut bob, &tick.outbound_lines, now);
        assert_eq!(flush_bob(&mut server, &mut bob, now), 0);
    }
    assert!(!server.room_playback_state(&room).paused);
}

#[test]
fn fresh_buffering_renewals_still_honor_the_maximum_pause() {
    let (mut server, mut bob, room) = buffering_fixture();
    for second in 1..=35 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        let mut sample = transport(second, true);
        sample.logical_pause = Some(server.room_playback_state(&room).paused);
        bob.observe_external_player_transport(sample, now);
        flush_bob(&mut server, &mut bob, now);
        let tick = server.collect_dispatch_at(now).unwrap();
        deliver_to_bob(&mut bob, &tick.outbound_lines, now);
        assert_eq!(server.room_playback_state(&room).paused, second < 31);
    }
    assert!(server.room_buffering_controls[&room].fail_open_latched);
}

#[test]
fn fresh_explicit_buffering_reports_preserve_pause_control() {
    let (mut server, mut bob, room) = buffering_fixture();
    for second in 1..=5 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        let payload = sorotte_protocol::StatePayload::new().with_playback_barrier_v1(
            sorotte_protocol::PlaybackBarrierStateExtension::new().with_transport(
                sorotte_protocol::TransportBufferingReportPayload::new(1, true)
                    .with_buffered_seconds(0.0)
                    .with_observed_at(now),
            ),
        );
        let replies = server
            .handle_line_fanout(
                "bob-client",
                &sorotte_protocol::encode_message_line(&ProtocolMessage::state(payload)).unwrap(),
            )
            .unwrap();
        deliver_to_bob(&mut bob, &replies, now);
        server.collect_dispatch_at(now).unwrap();
    }
    assert!(server.room_playback_state(&room).paused);
    assert_eq!(server.room_buffering_controls[&room].reports.len(), 1);
}

#[test]
fn selection_during_buffering_pause_retires_its_reports_and_resume_deadline() {
    let (mut server, mut bob, room) = buffering_fixture();
    server.set_clock_overrides_seconds(Some(101.0), Some(1.0));
    bob.observe_external_player_transport(transport(1, true), 101.0);
    flush_bob(&mut server, &mut bob, 101.0);
    assert!(server.room_buffering_controls[&room].paused_by_policy);
    server.handle_line_fanout("alice-client", r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"]},"playlistIndex":{"index":1}}}"#).unwrap();
    assert!(server.room_buffering_controls[&room].retired_for_selection);
    // A queued predecessor recovery report cannot release the successor pause.
    bob.observe_external_player_transport(transport(2, false), 102.0);
    flush_bob(&mut server, &mut bob, 102.0);
    assert!(server.room_buffering_controls[&room].reports.is_empty());
    for second in [4, 31, 40] {
        server
            .set_clock_overrides_seconds(Some(100.0 + f64::from(second)), Some(f64::from(second)));
        server
            .collect_dispatch_at(100.0 + f64::from(second))
            .unwrap();
        assert!(server.room_playback_state(&room).paused);
    }
    assert_eq!(server.room_playback_state(&room).position, 0.0);
}

fn assert_player_failure_updates_readiness(coordinated_start: bool) {
    let room = "standard-readiness";
    let mut server = ServerRuntime::default();
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let hello = server.handle_line_fanout(
        "bob-client",
        r#"{"Hello":{"username":"bob","room":{"name":"standard-readiness"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#,
    ).unwrap();
    let mut bob = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    deliver_to_bob(&mut bob, &hello, 100.0);
    assert!(bob.run_toggle_ready(true).unwrap());
    flush_bob(&mut server, &mut bob, 100.0);
    assert_eq!(
        server.room_readiness[room].participants["bob"]
            .record
            .user_intent,
        sorotte_protocol::UserReadinessIntent::Ready,
    );
    if coordinated_start {
        bob.set_playback_barrier_start_config(sorotte_client_core::PlaybackBarrierStartConfig {
            policy: Some(sorotte_protocol::PlaybackBarrierPolicy::AllEligible),
            ..sorotte_client_core::PlaybackBarrierStartConfig::default()
        });
    }
    let paused_before_prepare = server.room_playback_state(room).paused;
    bob.prepare_playback_media(
        LogicalMediaId::new("standard-readiness-media").unwrap(),
        MediaTransportKind::NetworkVod,
        100.0,
    );
    flush_bob(&mut server, &mut bob, 100.0);
    let room_generation = server.room_buffering_controls[room].config.media_generation;
    assert!(room_generation > 0);
    if !coordinated_start {
        assert!(!server.room_playback_barriers.contains_key(room));
        assert_eq!(
            server.room_readiness[room].start_gate_phase,
            RoomStartGatePhase::Inactive
        );
        assert_eq!(
            server.room_readiness[room].pause_owner,
            RoomPauseOwner::None
        );
        assert_eq!(
            server.room_playback_state(room).paused,
            paused_before_prepare
        );
    }
    server.set_clock_overrides_seconds(Some(101.0), Some(1.0));
    let mut failed = transport(1, false);
    failed.phase = Some(PlayerTransportPhase::Failed);
    bob.observe_external_player_transport(failed, 101.0);
    flush_bob(&mut server, &mut bob, 101.0);
    let readiness = &server.room_readiness[room];
    let record = &readiness.participants["bob"].record;
    assert_eq!(
        record.user_intent,
        sorotte_protocol::UserReadinessIntent::Ready
    );
    assert!(
        matches!(
            record.technical_state,
            sorotte_protocol::TechnicalPlayability::TerminallyBlocked { .. }
        ),
        "the shared readiness projection must expose an observed player failure under either Standard or coordinated start"
    );
    assert!(!record.room_ready);
    server.set_clock_overrides_seconds(Some(102.0), Some(2.0));
    bob.observe_external_player_transport(transport(2, false), 102.0);
    flush_bob(&mut server, &mut bob, 102.0);
    let recovered = &server.room_readiness[room].participants["bob"].record;
    assert!(matches!(
        recovered.technical_state,
        TechnicalPlayability::Playable { .. }
    ));
    assert!(recovered.terminal_technical_block.is_none());
    assert!(recovered.room_ready);
    assert_eq!(recovered.user_intent, UserReadinessIntent::Ready);
    if !coordinated_start {
        assert_eq!(
            server.room_readiness[room].start_gate_phase,
            RoomStartGatePhase::Inactive
        );
        assert_eq!(
            server.room_playback_state(room).paused,
            paused_before_prepare
        );
    }
}

#[test]
fn standard_start_exposes_terminal_player_failure_in_readiness() {
    assert_player_failure_updates_readiness(false);
}

#[test]
fn coordinated_start_exposes_terminal_player_failure_control() {
    assert_player_failure_updates_readiness(true);
}

#[test]
fn immediate_start_does_not_relabel_predecessor_observations() {
    assert_immediate_selection_does_not_relabel_predecessor(1);
}

#[test]
fn immediate_same_row_replay_requires_successor_media_preparation() {
    assert_immediate_selection_does_not_relabel_predecessor(0);
}

fn assert_immediate_selection_does_not_relabel_predecessor(successor_index: usize) {
    let room = "immediate-selection-readiness";
    let mut server = ServerRuntime::default();
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let mut alice = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    let mut bob = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    for (client_id, username, client) in [
        ("alice-client", "alice", &mut alice),
        ("bob-client", "bob", &mut bob),
    ] {
        let replies = server.handle_line_fanout(client_id, &format!(
            r#"{{"Hello":{{"username":"{username}","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}}}}"#,
        )).unwrap();
        deliver_to_client(client_id, client, &replies, 100.0);
    }
    let selection = server.handle_line_fanout("alice-client", r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"]},"playlistIndex":{"index":0}}}"#).unwrap();
    deliver_to_client("alice-client", &mut alice, &selection, 100.0);
    deliver_to_bob(&mut bob, &selection, 100.0);
    bob.prepare_playback_media(
        LogicalMediaId::new("episode-a.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        100.0,
    );
    flush_bob(&mut server, &mut bob, 100.0);
    bob.observe_external_player_transport(transport(1, false), 101.0);
    flush_bob(&mut server, &mut bob, 101.0);
    assert!(matches!(
        server.room_readiness[room].participants["bob"]
            .record
            .technical_state,
        TechnicalPlayability::Playable { .. }
    ));
    let predecessor_generation = server.room_readiness[room].media_generation.unwrap();

    let selection = server
        .handle_line_fanout(
            "alice-client",
            &format!(r#"{{"Set":{{"playlistIndex":{{"index":{successor_index}}}}}}}"#),
        )
        .unwrap();
    deliver_to_client("alice-client", &mut alice, &selection, 102.0);
    deliver_to_bob(&mut bob, &selection, 102.0);
    // Alice resolves B first. Bob's ordinary asynchronous resolver still has
    // A loaded, with fresh observations from that unchanged physical player.
    let successor_id = if successor_index == 0 {
        "episode-a.mkv"
    } else {
        "episode-b.mkv"
    };
    alice.prepare_playback_media(
        LogicalMediaId::new(successor_id).unwrap(),
        MediaTransportKind::LocalFile,
        102.0,
    );
    while let Some(pending) = alice.pending_protocol_line().unwrap() {
        let line = pending.line().to_owned();
        alice.acknowledge_protocol_line(pending.lease()).unwrap();
        let replies = server.handle_line_fanout("alice-client", &line).unwrap();
        deliver_to_client("alice-client", &mut alice, &replies, 102.0);
        deliver_to_bob(&mut bob, &replies, 102.0);
    }
    let successor_generation = server.room_readiness[room].media_generation.unwrap();
    assert_ne!(successor_generation, predecessor_generation);
    assert!(matches!(
        server.room_readiness[room].participants["bob"].record.technical_state,
        TechnicalPlayability::Preparing { media_generation } if media_generation == successor_generation
    ));
    for phase in [PlayerTransportPhase::Playing, PlayerTransportPhase::Failed] {
        let mut predecessor = transport(
            if phase == PlayerTransportPhase::Playing {
                3
            } else {
                4
            },
            false,
        );
        predecessor.phase = Some(phase);
        bob.observe_external_player_transport(predecessor, 104.0);
        flush_bob(&mut server, &mut bob, 104.0);
        assert!(
            matches!(
                server.room_readiness[room].participants["bob"].record.technical_state,
                TechnicalPlayability::Preparing { media_generation } if media_generation == successor_generation
            ),
            "predecessor {phase:?} must not be attributed to successor media"
        );
    }

    bob.prepare_playback_media_for_room_participation(
        LogicalMediaId::new(successor_id).unwrap(),
        MediaTransportKind::LocalFile,
        105.0,
    );
    let mut successor = transport(5, false);
    successor.media_generation = Some(PlayerMediaGeneration::new(2));
    bob.observe_external_player_transport(successor, 105.0);
    flush_bob(&mut server, &mut bob, 105.0);
    assert!(matches!(
        server.room_readiness[room].participants["bob"].record.technical_state,
        TechnicalPlayability::Playable { media_generation } if media_generation == successor_generation
    ));
    assert!(!server.room_playback_barriers.contains_key(room));
    assert_eq!(
        server.room_readiness[room].start_gate_phase,
        RoomStartGatePhase::Inactive
    );
}
