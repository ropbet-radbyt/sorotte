use super::*;

fn replay_then_edit(edit: bool, retire_reset: bool) -> (MpvLifecycleVerificationHarness, Runtime) {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":3}}}"#,
        )
        .unwrap();
    assert!(runtime.session().has_pending_playlist_index_reset_intent());
    if edit {
        runtime.session_mut().apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"bob","sorottePlaylistEpoch":4}}}"#,
        ).unwrap();
    }
    // Mirrors ClientApplication::synchronize_canonical_playlist_to_player:
    // accepted SetPosition, accepted SetPaused, mark applied, then retire once
    // the post-selection State is present. RuntimePlayer::set_position returns
    // on tracked-command acceptance; its semantic receipt remains outstanding.
    accept_replay_seek(&mut harness, &mut runtime, 0.0);
    let pause_command_id = harness.accept_tracked_pause();
    runtime.with_player_io(|player| player.pause_command_id = Some(pause_command_id));
    runtime.player_mut().set_paused(true).unwrap();
    assert!(
        runtime
            .session_mut()
            .mark_pending_playlist_index_reset_physical_effect_applied(1)
    );
    if retire_reset {
        runtime.session_mut().apply_message_json(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        ).unwrap();
        assert!(
            runtime
                .session_mut()
                .complete_pending_playlist_index_reset_for_attachment(1)
                .is_some()
        );
    }
    (harness, runtime)
}

fn end_predecessor(harness: &mut MpvLifecycleVerificationHarness, runtime: &mut Runtime) -> bool {
    harness
        .ingest_decoded_mpv_json(json!({"event":"end-file","playlist_entry_id":77,"reason":"eof"}));
    apply_and_ack(harness, runtime, 2.0);
    let advanced = runtime
        .run_advance_playlist_after_natural_completion()
        .unwrap();
    println!(
        "advanced={advanced} selected={:?}",
        runtime.session().current_room_playlist().unwrap().index
    );
    advanced
}

#[test]
fn replay_without_edit_rejects_predecessor_eof() {
    let (mut harness, mut runtime) = replay_then_edit(false, true);
    assert!(!end_predecessor(&mut harness, &mut runtime));
}

#[test]
fn unretired_reset_rejects_predecessor_eof_after_edit() {
    let (mut harness, mut runtime) = replay_then_edit(true, false);
    assert!(!end_predecessor(&mut harness, &mut runtime));
}

#[test]
fn replay_then_unselected_edit_cannot_adopt_unobserved_reset() {
    let (mut harness, mut runtime) = replay_then_edit(true, true);
    assert!(
        !end_predecessor(&mut harness, &mut runtime),
        "an unrelated edit cannot make an unobserved replay reset own predecessor EOF"
    );
}

#[test]
fn matching_replay_seek_receipt_allows_completion_after_edit() {
    let (mut harness, mut runtime) = replay_then_edit(true, true);
    for event in [
        json!({"event":"seek"}),
        json!({"event":"property-change","name":"eof-reached","data":false}),
        json!({"event":"property-change","name":"time-pos","data":0.0}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"core-idle","data":false}),
        json!({"event":"property-change","name":"pause","data":false}),
        json!({"event":"property-change","name":"time-pos","data":239.65}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
    apply_and_ack(&mut harness, &mut runtime, 1.5);
    assert!(end_predecessor(&mut harness, &mut runtime));
}

fn decoded_mpv_seek(
    fresh_target_headroom: Option<f64>,
) -> (
    MpvLifecycleVerificationHarness,
    Runtime,
    sorotte_client_core::PlaybackCoordinationSnapshot,
) {
    use sorotte_client_core::{
        CoordinatorPlayerCommand, LogicalMediaId, MediaTransportKind, PlaybackCoordinatorAction,
    };
    let path = "https://media.example.invalid/episode1.mkv";
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let mut runtime = ClientRuntime::new(
        session,
        CompletionTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("network-episode").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    let mut harness = MpvLifecycleVerificationHarness::new();
    harness.accept_tracked_load(path, []);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            77,
            Some(path.into()),
            true,
        )],
        Some(path.into()),
    );
    for event in [
        json!({"event":"start-file","playlist_entry_id":77}),
        json!({"event":"property-change","name":"path","data":path}),
        json!({"event":"property-change","name":"duration","data":240.0}),
        json!({"event":"file-loaded"}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"pause","data":true}),
        json!({"event":"property-change","name":"paused-for-cache","data":false}),
        json!({"event":"property-change","name":"seekable","data":true}),
        json!({"event":"property-change","name":"time-pos","data":5.0}),
        json!({"event":"property-change","name":"cache-buffering-state","data":100.0}),
        json!({"event":"property-change","name":"demuxer-cache-state","data":{
            "seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,
            "raw-input-rate":9_000_000,
        }}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    runtime.session_mut().apply_message_json_at(
        r#"{"State":{"playstate":{"position":40.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        1.1,
    ).unwrap();
    let actions = runtime.reconcile_external_player_playback(1.1);
    let command_id = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPosition(40.0),
            } => Some(*command_id),
            _ => None,
        })
        .expect("the cold seek must have been dispatched");
    harness.accept_tracked_seek(40.0);
    runtime.report_external_coordinator_command_dispatch(command_id, Ok(()), 1.1);
    // These decoded mpv events exercise handle_seek_event, which clears the
    // adapter's cache evidence, followed by actual sparse transport emission.
    for event in [
        json!({"event":"seek"}),
        json!({"event":"property-change","name":"time-pos","data":40.0}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
    if let Some(headroom) = fresh_target_headroom {
        harness.ingest_decoded_mpv_json(json!({"event":"property-change","name":"cache-buffering-state","data":if headroom > 0.0 {100.0} else {0.0}}));
        harness.ingest_decoded_mpv_json(
            json!({"event":"property-change","name":"demuxer-cache-state","data":{
                "seekable-ranges":[{"start":40.0,"end":40.0 + headroom}],"cache-duration":headroom,
            }}),
        );
    }
    harness
        .ingest_decoded_mpv_json(json!({"event":"property-change","name":"seeking","data":false}));
    let batch = harness.take_event_batch().expect("seek telemetry");
    let deltas = batch
        .events
        .iter()
        .filter_map(|event| match &event.event {
            sorotte_player_api::PlayerEvent::TransportDelta(delta) => Some(delta),
            _ => None,
        })
        .collect::<Vec<_>>();
    if fresh_target_headroom.is_none() {
        assert!(
            deltas
                .iter()
                .all(|delta| delta.buffered_duration_seconds.is_none()
                    && delta.cache_percentage.is_none()
                    && delta.input_rate_bytes_per_second.is_none())
        );
    }
    println!("fresh_target_headroom={fresh_target_headroom:?} decoded seek deltas={deltas:?}");
    runtime
        .apply_ordered_player_event_batch_for_verification(&batch, 1.2)
        .unwrap();
    let snapshot = runtime.playback_coordination_snapshot();
    println!(
        "decoded seek headroom={:?} terminal={:?} applied_revision={:?}",
        snapshot.metrics.last_buffered_ahead_seconds,
        snapshot.last_seek_preparation_terminal_outcome,
        snapshot.last_applied_revision
    );
    harness.acknowledge(batch.acknowledgement_token).unwrap();
    runtime.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
    (harness, runtime, snapshot)
}

#[test]
fn decoded_mpv_seek_cannot_reuse_preseek_cache() {
    let (mut harness, mut runtime, snapshot) = decoded_mpv_seek(None);
    assert_eq!(
        snapshot.metrics.last_buffered_ahead_seconds, None,
        "the real mpv decoder's cleared cache epoch must not reuse the preceding 10s headroom"
    );
    assert_eq!(
        snapshot.last_seek_preparation_terminal_outcome, None,
        "sparse seek completion without fresh target headroom must remain preparing"
    );
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change","name":"cache-buffering-state","data":100.0}),
    );
    harness.ingest_decoded_mpv_json(
        json!({"event":"property-change","name":"demuxer-cache-state","data":{
            "seekable-ranges":[{"start":40.0,"end":46.0}],"cache-duration":6.0,
        }}),
    );
    apply_and_ack(&mut harness, &mut runtime, 1.3);
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .last_seek_preparation_terminal_outcome,
        Some(sorotte_client_core::SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn fresh_empty_target_cache_keeps_seek_preparing() {
    let (_, _, snapshot) = decoded_mpv_seek(Some(0.0));
    assert_eq!(snapshot.last_seek_preparation_terminal_outcome, None);
    assert!(snapshot.seek_preparation.is_some());
}

#[test]
fn fresh_sufficient_target_cache_completes_preparation() {
    let (_, _, snapshot) = decoded_mpv_seek(Some(6.0));
    assert_eq!(
        snapshot.last_seek_preparation_terminal_outcome,
        Some(sorotte_client_core::SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(snapshot.metrics.last_buffered_ahead_seconds, Some(6.0));
}
