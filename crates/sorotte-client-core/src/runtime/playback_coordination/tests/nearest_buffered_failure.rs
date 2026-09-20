use super::*;

type NearestRuntime = ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl>;

fn sample(runtime: &mut NearestRuntime, sequence: u64, now: f64, position: f64, buffering: bool) {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(1);
    let mut transport = ordered_paused_transport(
        generation,
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(now)),
        position,
    );
    transport.phase = SnapshotField::Known(if buffering {
        PlayerTransportPhase::Rebuffering
    } else {
        PlayerTransportPhase::ReadyPaused
    });
    transport.paused_for_cache = SnapshotField::Known(buffering);
    transport.core_idle = SnapshotField::Known(true);
    transport.seekable = SnapshotField::Known(true);
    transport.seekable_ranges =
        SnapshotField::Known(vec![sorotte_player_api::PlayerSeekableRange::new(
            0.0, 35.0,
        )]);
    runtime.player.ordered_batches.push_back(ordered_batch(
        epoch,
        sequence,
        sequence,
        Some(active_snapshot(
            epoch,
            sequence,
            LoadAttemptId::new(1),
            generation,
            transport,
        )),
        Vec::new(),
        Vec::new(),
    ));
    runtime.drain_player_transport_coordination(now).unwrap();
}

fn room_seek(runtime: &mut NearestRuntime, position: f64, revision: u64, now: f64) {
    runtime
        .session_mut()
        .apply_message_json_at(
            &serde_json::json!({
                "State": {"playstate": {"position": position, "paused": false,
                    "doSeek": true, "setBy": "bob", "sorotteTransportRevision": revision}}
            })
            .to_string(),
            now,
        )
        .unwrap();
    runtime.drain_player_transport_coordination(now).unwrap();
}

fn runtime_fixture() -> NearestRuntime {
    let mut runtime = ClientRuntime::new(
        participant_status_session(),
        CoordinatedTestPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("round2-nearest-runtime").unwrap(),
        MediaTransportKind::NetworkVod,
        0.0,
    );
    sample(&mut runtime, 1, 0.1, 5.0, true);
    room_seek(&mut runtime, 40.0, 1, 0.2);
    assert!(
        runtime
            .playback_coordination_snapshot()
            .seek_preparation
            .unwrap()
            .can_join_nearest_buffered
    );
    assert!(
        !runtime
            .player
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPosition(_)))
    );
    runtime
}

#[test]
fn runtime_rejected_nearest_join_does_not_retry_cold_target() {
    let mut runtime = runtime_fixture();
    runtime.player.reject_seek_commands = true;
    // This is the public request called by the CLI application owner. Its
    // production executor reports the actual tracked adapter dispatch error.
    let error = runtime
        .run_join_nearest_buffered_seek_preparation(0.3)
        .unwrap_err();
    assert!(matches!(error, PlayerError::OperationFailed(_)));
    assert_eq!(
        runtime.player.commands.last(),
        Some(&PlayerCommand::SetPosition(35.0))
    );
    runtime.player.reject_seek_commands = false;
    runtime.player.commands.clear();
    sample(&mut runtime, 2, 3.0, 5.0, false);
    let unwanted: Vec<f64> = runtime
        .player
        .commands
        .iter()
        .filter_map(|command| match command {
            PlayerCommand::SetPosition(position) => Some(*position),
            _ => None,
        })
        .collect();
    println!("runtime seek targets after rejected Join nearest: {unwanted:?}");
    // New explicit room intent is still usable after the rejected alternative.
    runtime.player.commands.clear();
    room_seek(&mut runtime, 20.0, 2, 3.1);
    sample(&mut runtime, 3, 3.2, 5.0, false);
    assert!(
        runtime
            .player
            .commands
            .iter()
            .any(|command| matches!(command,
        PlayerCommand::SetPosition(position) if (19.0..=21.0).contains(position)))
    );
    assert!(
        !unwanted.iter().any(|position| *position > 35.0),
        "a rejected buffered alternative must not restore the cold room seek: {unwanted:?}"
    );
}

#[test]
fn runtime_successful_nearest_join_and_next_seek() {
    let mut runtime = runtime_fixture();
    assert!(
        runtime
            .run_join_nearest_buffered_seek_preparation(0.3)
            .unwrap()
    );
    assert_eq!(
        runtime.player.commands.last(),
        Some(&PlayerCommand::SetPosition(35.0))
    );
    runtime.player.commands.clear();
    sample(&mut runtime, 2, 0.4, 35.0, false);
    sample(&mut runtime, 3, 0.5, 35.0, false);
    assert!(
        !runtime
            .player
            .commands
            .iter()
            .any(|command| matches!(command, PlayerCommand::SetPosition(_))),
        "successful Join nearest must not restore the cold target: {:?}",
        runtime.player.commands
    );
    runtime.player.commands.clear();
    room_seek(&mut runtime, 20.0, 2, 0.6);
    sample(&mut runtime, 4, 0.7, 35.0, false);
    assert!(
        runtime
            .player
            .commands
            .iter()
            .any(|command| matches!(command,
        PlayerCommand::SetPosition(position) if (19.0..=21.0).contains(position)))
    );
}
