use super::*;

fn feed(
    coordination: &mut RuntimePlaybackCoordination,
    snapshot: &mut PlayerTransportSnapshot,
    update: PlayerTransportTelemetryUpdate,
    now: f64,
    ordered: bool,
) -> Vec<PlaybackCoordinatorAction> {
    if ordered {
        let delta = PlayerTransportDelta::from(update);
        snapshot.apply_delta(delta.clone());
        coordination.observe_ordered_transport_delta(snapshot, &delta, now)
    } else {
        coordination.observe_transport(update, now)
    }
}

#[test]
fn sparse_ordered_position_must_not_reuse_preseek_cache() {
    for ordered in [false, true] {
        let mut coordination = RuntimePlaybackCoordination::default();
        let generation = coordination
            .prepare_media(
                LogicalMediaId::new("audit-cold-seek").unwrap(),
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        let mut snapshot = PlayerTransportSnapshot::default();
        let mut initial = paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 5.0);
        initial.seekable_ranges = Some(vec![sorotte_player_api::PlayerSeekableRange::new(
            0.0, 10.0,
        )]);
        initial.cache_buffering_percent = Some(100.0);
        initial.buffered_ahead_seconds = Some(10.0);
        initial.input_rate_bytes_per_second = Some(9_000_000);
        feed(&mut coordination, &mut snapshot, initial, 0.1, ordered);
        coordination
            .coordinator
            .update_desired_room_state_with_kind(
                DesiredRoomPlayback {
                    media_generation: generation,
                    state_revision: 1,
                    paused: true,
                    anchor_position_seconds: 40.0,
                    anchor_observed_at_seconds: 0.1,
                    force_seek: true,
                },
                DesiredRoomPlaybackUpdateKind::ExplicitSeek,
            );
        let actions = feed(
            &mut coordination,
            &mut snapshot,
            paused_transport(1, 0.2, PlayerTransportPhase::ReadyPaused, 5.0),
            0.2,
            ordered,
        );
        assert!(actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(40.0),
                ..
            }
        )));
        let mut at_target = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(0.3)),
        );
        at_target.position_seconds = Some(40.0);
        at_target.seeking = Some(false);
        let actions = feed(&mut coordination, &mut snapshot, at_target, 0.3, ordered);
        println!(
            "ordered={ordered} snapshot={:?} actions={actions:?}",
            coordination.snapshot()
        );
        assert_eq!(
            coordination.snapshot().metrics.last_buffered_ahead_seconds,
            None,
            "old cache headroom cannot be reassigned to a new seek target (ordered={ordered})"
        );
        assert_eq!(
            coordination
                .snapshot()
                .last_seek_preparation_terminal_outcome,
            None,
            "one position sample cannot prove an unbuffered target ready (ordered={ordered})"
        );
    }
}
