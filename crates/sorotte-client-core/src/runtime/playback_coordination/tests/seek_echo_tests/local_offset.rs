use super::*;
use sorotte_player_api::PlayerSeekableRange;

#[test]
fn sparse_player_deltas_keep_offset_positions_ranges_and_real_observation_clocks() {
    let mut fixture = SeekFixture::new();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.1)
        .unwrap();
    let epoch = PlayerAttachmentEpoch::new(1);
    let mut delta = PlayerTransportDelta::from(transport(
        fixture.generation.get(),
        1.2,
        PlayerTransportPhase::ReadyPaused,
        8.0,
    ));
    delta.load_attempt_id = Some(LoadAttemptId::new(1));
    delta.seekable_ranges = Some(vec![PlayerSeekableRange::new(10.0, 20.0)]);
    delta.known_live_seekable_window = Some(PlayerSeekableRange::new(10.0, 20.0));
    fixture
        .runtime
        .player
        .ordered_batches
        .push_back(ordered_batch(
            epoch,
            2,
            2,
            None,
            vec![SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::TransportDelta(delta),
            }],
            Vec::new(),
        ));
    fixture
        .runtime
        .drain_player_transport_coordination(1.2)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(3.0)
    );
    let before = fixture
        .runtime
        .playback_coordination
        .latest_observation
        .clone()
        .unwrap();
    assert_eq!(
        before.seekable_ranges,
        Some(vec![PlayerSeekableRange::new(5.0, 15.0)])
    );
    assert_eq!(
        before.known_live_seekable_window,
        Some(PlayerSeekableRange::new(5.0, 15.0))
    );

    fixture
        .runtime
        .set_local_playback_offset_seconds(-2.0, 1.3)
        .unwrap();
    let after = fixture
        .runtime
        .playback_coordination
        .latest_observation
        .as_ref()
        .unwrap();
    assert_eq!(after.position_seconds, Some(10.0));
    assert_eq!(
        after.seekable_ranges,
        Some(vec![PlayerSeekableRange::new(12.0, 22.0)])
    );
    assert_eq!(
        after.known_live_seekable_window,
        Some(PlayerSeekableRange::new(12.0, 22.0))
    );
    assert_eq!(
        after.observed_at_seconds, before.observed_at_seconds,
        "changing coordinates must not refresh old physical evidence"
    );

    let mut positionless = PlayerTransportDelta::from(PlayerTransportTelemetryUpdate::new(
        fixture.generation,
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.4)),
    ));
    positionless.load_attempt_id = Some(LoadAttemptId::new(1));
    positionless.playback_rate = Some(1.0);
    fixture
        .runtime
        .player
        .ordered_batches
        .push_back(ordered_batch(
            epoch,
            3,
            3,
            None,
            vec![SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::TransportDelta(positionless),
            }],
            Vec::new(),
        ));
    fixture
        .runtime
        .drain_player_transport_coordination(1.4)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(10.0),
        "a positionless update cannot restore the previous offset coordinates"
    );
    assert_eq!(
        fixture
            .runtime
            .playback_coordination
            .latest_observation
            .as_ref()
            .unwrap()
            .seekable_ranges,
        Some(vec![PlayerSeekableRange::new(12.0, 22.0)])
    );
}

#[test]
fn offset_after_local_seek_preserves_the_target_until_server_acknowledgement() {
    let mut fixture = SeekFixture::new();
    fixture.emit_seek();
    fixture.runtime.player.commands.clear();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.1)
        .unwrap();
    assert_eq!(
        fixture.runtime.player.commands.last(),
        Some(&PlayerCommand::SetPosition(16.0))
    );
    assert!(
        fixture
            .runtime
            .deliver_queued_protocol_messages()
            .is_empty()
    );
}

#[test]
fn offset_after_newer_room_authority_does_not_resurrect_an_unacknowledged_seek() {
    let mut fixture = SeekFixture::new();
    fixture.emit_seek();
    fixture.reconcile(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(20.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("bob")
                    .with_transport_revision(35),
            )
            .with_ignoring_on_the_fly(
                IgnoringOnTheFlyPayload::new()
                    .with_server(1)
                    .with_client(fixture.seek_counter),
            ),
    );
    assert_eq!(
        fixture
            .runtime
            .session()
            .current_room_playstate()
            .unwrap()
            .position,
        Some(20.0)
    );
    fixture.runtime.player.commands.clear();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.4)
        .unwrap();
    assert_eq!(
        fixture.runtime.player.commands.last(),
        Some(&PlayerCommand::SetPosition(25.0))
    );
    assert!(
        fixture
            .runtime
            .deliver_queued_protocol_messages()
            .is_empty()
    );
}

#[test]
fn local_offset_keeps_heartbeat_and_later_seek_in_room_coordinates() {
    let mut fixture = SeekFixture::new();
    fixture.runtime.player.commands.clear();
    assert!(
        fixture
            .runtime
            .set_local_playback_offset_seconds(5.0, 1.1)
            .unwrap()
    );
    assert!(
        fixture
            .runtime
            .player
            .commands
            .contains(&PlayerCommand::SetPosition(5.0))
    );
    assert!(
        fixture
            .runtime
            .deliver_queued_protocol_messages()
            .is_empty()
    );

    fixture.queue_observation(true, 5.0);
    assert!(fixture.runtime.run_state_sync_heartbeat_with_ping(false));
    let heartbeat = fixture.take_response();
    let playstate = heartbeat
        .playstate
        .expect("ordinary heartbeat remains publishable");
    assert_eq!(playstate.position, Some(0.0));
    assert_ne!(playstate.do_seek, Some(true));

    fixture.runtime.player.commands.clear();
    fixture.emit_seek();
    assert!(
        fixture
            .runtime
            .player
            .commands
            .contains(&PlayerCommand::SetPosition(16.0))
    );
    fixture.queue_observation(true, 16.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.3)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(11.0)
    );
}

#[test]
fn canonical_remote_seek_is_applied_with_the_local_offset() {
    let mut fixture = SeekFixture::new();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.1)
        .unwrap();
    fixture.queue_observation(true, 5.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.2)
        .unwrap();
    fixture.runtime.player.commands.clear();
    fixture.reconcile(
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(20.0)
                .with_paused(true)
                .with_do_seek(true)
                .with_set_by("bob")
                .with_transport_revision(35),
        ),
    );
    fixture.queue_observation(true, 5.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.4)
        .unwrap();
    assert!(
        fixture
            .runtime
            .player
            .commands
            .contains(&PlayerCommand::SetPosition(25.0)),
        "a remote room seek must enter the offset physical timeline"
    );
}

#[test]
fn rejected_offset_retains_previous_coordinates_and_emits_no_room_seek() {
    let mut fixture = SeekFixture::new();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.1)
        .unwrap();
    fixture.queue_observation(true, 5.0);
    fixture.runtime.player.reject_seek_commands = true;
    assert!(
        fixture
            .runtime
            .set_local_playback_offset_seconds(8.0, 1.2)
            .is_err()
    );
    assert_eq!(fixture.runtime.local_playback_offset_seconds(), 5.0);
    assert!(
        fixture
            .runtime
            .deliver_queued_protocol_messages()
            .is_empty()
    );
    fixture.queue_observation(true, 5.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.3)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(0.0)
    );
}

#[test]
fn transport_reconnect_preserves_local_offset() {
    let mut fixture = SeekFixture::new();
    fixture
        .runtime
        .set_local_playback_offset_seconds(5.0, 1.1)
        .unwrap();
    fixture
        .runtime
        .session_mut()
        .reset_sync_state_for_reconnect();
    assert_eq!(fixture.runtime.local_playback_offset_seconds(), 5.0);
    fixture
        .runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    fixture.queue_observation(true, 7.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.3)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(2.0)
    );
}
