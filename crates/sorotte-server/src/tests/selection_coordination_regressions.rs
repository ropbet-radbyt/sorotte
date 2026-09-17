use super::*;
use sorotte_client_app::app_boundary::application::ClientApplication;
use sorotte_client_core::ClientSession;
use sorotte_player_api::DisconnectedPlayer;
use sorotte_protocol::{
    DirectReadinessSurface, MediaReadyPayload, PlaybackBarrierPhase, PlaybackBarrierStateExtension,
    ReadinessIntentRequest, ReadinessSetExtension, ReadinessStateExtension, RoomStartGatePhase,
    StatePayload, TechnicalPlayability, TechnicalPlayabilityPhase, TechnicalReadinessReport,
    UserReadinessIntent, UserReadinessMutationSource, encode_message_line,
};

fn join(runtime: &mut ServerRuntime, client: &str, username: &str) -> Vec<DirectedOutboundLine> {
    runtime
        .handle_line_fanout(
            client,
            &json!({"Hello": {
                "username": username, "room": {"name": "room"}, "version": "1.7.5",
                "features": {"sorotteReadinessV2": true, "sorottePlaybackBarrierV1": true}
            }})
            .to_string(),
        )
        .unwrap()
}

fn intent(runtime: &mut ServerRuntime, client: &str, nonce: u64) -> Vec<DirectedOutboundLine> {
    let session = &runtime.sessions[client];
    let epoch = runtime.room_readiness[&session.room].participants[&session.username]
        .record
        .membership_epoch;
    let request = ReadinessIntentRequest::new(
        format!("{client}-ready-{nonce}"),
        nonce,
        epoch,
        UserReadinessIntent::Ready,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton,
        },
    );
    let line = encode_message_line(&ProtocolMessage::set(
        SetPayload::new().with_readiness_v2(ReadinessSetExtension::new().with_intent(request)),
    ))
    .unwrap();
    runtime.handle_line_fanout(client, &line).unwrap()
}

fn prepare(runtime: &mut ServerRuntime, nonce: u64, media: &str) -> Vec<DirectedOutboundLine> {
    runtime.handle_line_fanout("alice-client", &json!({"Set": {"sorottePlaybackBarrierV1": {
        "prepare": {
            "mediaGeneration": 0, "requestNonce": nonce, "requestId": format!("start-{nonce}"),
            "loadIntent": "newPlayback", "logicalMediaId": media, "targetPosition": 0.0,
            "policy": "allEligible", "timeoutMs": 30000, "timeoutAction": "askController"
        }
    }}}).to_string()).unwrap()
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
    runtime
        .handle_line_fanout(client, &encode_message_line(&message).unwrap())
        .unwrap()
}

fn preparing_room() -> ServerRuntime {
    let mut runtime = ServerRuntime::default();
    runtime.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    join(&mut runtime, "alice-client", "alice");
    join(&mut runtime, "bob-client", "bob");
    runtime.handle_line_fanout("alice-client", r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"]},"playlistIndex":{"index":0}}}"#).unwrap();
    intent(&mut runtime, "alice-client", 1);
    prepare(&mut runtime, 1, "logical:episode-a");
    playable(&mut runtime, "alice-client", 1);
    playable(&mut runtime, "bob-client", 1);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );
    assert!(runtime.room_playback_state("room").paused);
    runtime
}

fn has_commit(lines: &[DirectedOutboundLine]) -> bool {
    decode_directed_lines(lines)
        .into_iter()
        .any(|(_, message)| {
            matches!(message, ProtocolMessage::Set(set) if set.set.playback_barrier_v1().unwrap()
            .is_some_and(|extension| extension.commit.is_some()))
        })
}

#[test]
fn ready_after_selection_change_must_not_commit_predecessor() {
    let mut runtime = preparing_room();
    runtime
        .handle_line_fanout("bob-client", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .unwrap();
    assert_eq!(runtime.room_playlist_state("room").index, Some(1));
    assert_eq!(runtime.room_playback_state("room").position, 0.0);
    assert!(runtime.room_playback_state("room").paused);
    assert_eq!(runtime.room_readiness["room"].media_generation, None);
    assert_eq!(
        runtime.room_readiness["room"].start_gate_phase,
        RoomStartGatePhase::Inactive
    );
    for participant in runtime.room_readiness["room"].participants.values() {
        assert_eq!(
            participant.record.technical_state,
            TechnicalPlayability::Unknown
        );
        assert!(!participant.record.start_eligible);
    }
    // In-flight predecessor reports and an exact prepare retry remain stale.
    assert!(!has_commit(&playable(&mut runtime, "bob-client", 1)));
    assert!(!has_commit(&prepare(&mut runtime, 1, "logical:episode-a")));
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Degraded
    );
    assert_eq!(runtime.room_readiness["room"].media_generation, None);
    join(&mut runtime, "charlie-client", "charlie");
    assert_eq!(runtime.room_readiness["room"].media_generation, None);
    let ready = intent(&mut runtime, "bob-client", 1);
    let playback = runtime.room_playback_state("room");
    assert!(
        !has_commit(&ready),
        "Ready for the new selection must not commit the predecessor's generation using old technical evidence"
    );
    assert!(playback.paused);
}

#[test]
fn ready_without_selection_change_commits_control() {
    let mut runtime = preparing_room();
    let ready = intent(&mut runtime, "bob-client", 1);
    assert!(has_commit(&ready));
    assert!(!runtime.room_playback_state("room").paused);
    assert_eq!(runtime.room_playback_state("room").position, 0.0);
}

#[test]
fn fresh_prepare_after_selection_change_commits_successor_control() {
    let mut runtime = preparing_room();
    runtime
        .handle_line_fanout("bob-client", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .unwrap();
    prepare(&mut runtime, 2, "logical:episode-b");
    assert!(!has_commit(&playable(&mut runtime, "bob-client", 1)));
    assert!(runtime.room_playback_state("room").paused);
    playable(&mut runtime, "alice-client", 2);
    playable(&mut runtime, "bob-client", 2);
    let ready = intent(&mut runtime, "bob-client", 1);
    assert!(has_commit(&ready));
    assert_eq!(
        runtime.room_playback_barriers["room"]
            .prepare
            .logical_media_id,
        "logical:episode-b"
    );
    assert!(!runtime.room_playback_state("room").paused);
    assert_eq!(runtime.room_playback_state("room").position, 0.0);
}

fn deliver_bob(app: &mut ClientApplication<DisconnectedPlayer>, lines: &[DirectedOutboundLine]) {
    for directed in lines.iter().filter(|line| line.client_id == "bob-client") {
        app.apply_protocol_line(&directed.line, 100.0, true, false, false)
            .unwrap();
    }
}

fn flush_bob(
    runtime: &mut ServerRuntime,
    app: &mut ClientApplication<DisconnectedPlayer>,
) -> Vec<DirectedOutboundLine> {
    let mut output = Vec::new();
    let mut count = 0;
    while let Some(pending) = app.pending_protocol_line().unwrap() {
        count += 1;
        assert!(count < 50);
        let line = pending.line().to_owned();
        app.acknowledge_protocol_line(pending.lease()).unwrap();
        let replies = runtime.handle_line_fanout("bob-client", &line).unwrap();
        deliver_bob(app, &replies);
        output.extend(replies);
    }
    output
}

#[test]
fn app_selection_then_ready_must_not_commit_retired_media() {
    let mut runtime = ServerRuntime::default();
    runtime.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let mut bob = ClientApplication::new(ClientSession::default(), DisconnectedPlayer);
    join(&mut runtime, "alice-client", "alice");
    let hello = join(&mut runtime, "bob-client", "bob");
    deliver_bob(&mut bob, &hello);
    flush_bob(&mut runtime, &mut bob);
    let playlist = runtime.handle_line_fanout("alice-client", r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-b.mkv"]},"playlistIndex":{"index":0}}}"#).unwrap();
    deliver_bob(&mut bob, &playlist);
    let intent = intent(&mut runtime, "alice-client", 1);
    deliver_bob(&mut bob, &intent);
    let prepare = prepare(&mut runtime, 1, "logical:episode-a");
    deliver_bob(&mut bob, &prepare);
    for client in ["alice-client", "bob-client"] {
        let playable = playable(&mut runtime, client, 1);
        deliver_bob(&mut bob, &playable);
    }
    flush_bob(&mut runtime, &mut bob);
    assert_eq!(
        runtime.room_playback_barriers["room"].phase,
        PlaybackBarrierPhase::Preparing
    );

    // These are the production app entry points. Successor media resolution
    // has not completed, so there is no new prepare or playable report for B.
    assert!(bob.run_set_playlist_index(1).unwrap());
    flush_bob(&mut runtime, &mut bob);
    assert_eq!(
        bob.session().current_room_playlist().unwrap().index,
        Some(1)
    );
    assert!(runtime.room_playback_state("room").paused);
    assert!(bob.run_set_ready_for_user("", true, true).unwrap());
    let ready = flush_bob(&mut runtime, &mut bob);
    assert!(
        !has_commit(&ready),
        "the normal app selection + Ready path must not start retired media A"
    );
}

#[test]
fn ready_after_playlist_clear_must_not_commit_retired_media() {
    let mut runtime = preparing_room();
    runtime
        .handle_line_fanout("bob-client", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .unwrap();
    assert!(runtime.room_playlist_state("room").files.is_empty());
    assert_eq!(runtime.room_playlist_state("room").index, None);
    let ready = intent(&mut runtime, "bob-client", 1);
    assert!(
        !has_commit(&ready),
        "clearing the selection must retire its preparing start lifecycle"
    );
}

#[test]
fn unselected_playlist_edit_preserves_current_gate_control() {
    let mut runtime = preparing_room();
    runtime
        .handle_line_fanout(
            "bob-client",
            r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","episode-c.mkv"]}}}"#,
        )
        .unwrap();
    let ready = intent(&mut runtime, "bob-client", 1);
    assert!(has_commit(&ready));
    assert_eq!(runtime.room_playlist_state("room").index, Some(0));
}
