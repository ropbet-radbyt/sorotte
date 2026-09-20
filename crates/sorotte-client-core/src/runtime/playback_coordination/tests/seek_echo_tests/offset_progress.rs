use super::*;

fn delayed_seek_fixture(paused: bool) -> (SeekFixture, f64) {
    let mut fixture = SeekFixture::with_paused(paused);
    fixture.emit_seek();
    let mut current_position = 11.0;
    // A two-second acknowledgement delay. Every tick is a new complete
    // physical snapshot, with its own adapter observation timestamp.
    for tick in 1..=20 {
        let now = 1.0 + f64::from(tick) / 10.0;
        current_position = if paused { 11.0 } else { 10.0 + now };
        fixture.queue_observation(paused, current_position);
        fixture
            .runtime
            .drain_player_transport_coordination(now)
            .unwrap();
    }
    assert_eq!(
        fixture.runtime.session().local_position_seconds(),
        Some(current_position)
    );
    let actual_position = fixture
        .runtime
        .playback_coordination
        .latest_position_observation
        .unwrap();
    assert_eq!(actual_position.position_seconds, current_position);
    assert_eq!(
        actual_position.last_actual_position_observed_at_seconds,
        3.0
    );
    assert!(
        fixture
            .runtime
            .playback_coordination
            .unacknowledged_local_seek
            .is_some()
    );
    assert_eq!(
        fixture.runtime.session().current_room_transport_revision(),
        Some(34)
    );
    (fixture, current_position)
}

fn acknowledge_current_seek(fixture: &mut SeekFixture, position: f64, paused: bool) {
    // The ordinary server acknowledgement enters through the same reconcile
    // boundary used by the connected runtime, with the emitted seek counter.
    let state = StatePayload::new()
        .with_playstate(
            PlaystatePayload::new()
                .with_position(position)
                .with_paused(paused)
                .with_do_seek(true)
                .with_set_by("alice")
                .with_transport_revision(35),
        )
        .with_ignoring_on_the_fly(
            IgnoringOnTheFlyPayload::new()
                .with_server(1)
                .with_client(fixture.seek_counter),
        );
    assert!(
        fixture
            .runtime
            .run_state_sync_reconcile_with_inbound_state_with_ping_at(state, false, 3.0)
    );
    fixture.runtime.deliver_queued_protocol_messages();
    assert!(
        fixture
            .runtime
            .playback_coordination
            .unacknowledged_local_seek
            .is_none()
    );
    assert_eq!(
        fixture.runtime.session().current_room_transport_revision(),
        Some(35)
    );
}

fn offset_target(fixture: &mut SeekFixture, offset: f64) -> f64 {
    fixture.runtime.player.commands.clear();
    fixture
        .runtime
        .set_local_playback_offset_seconds(offset, 3.0)
        .unwrap();
    fixture
        .runtime
        .player
        .commands
        .iter()
        .rev()
        .find_map(|command| match command {
            PlayerCommand::SetPosition(position) => Some(*position),
            _ => None,
        })
        .unwrap()
}

fn offset_after_unacknowledged_seek(paused: bool) -> (f64, f64) {
    let (mut fixture, current_position) = delayed_seek_fixture(paused);
    let target = offset_target(&mut fixture, 5.0);
    println!(
        "paused={paused} fresh_position_at_3s={current_position} server_revision_before_ack=34 offset_seek={target}"
    );
    // Follow-on operation remains usable once the canonical echo arrives.
    acknowledge_current_seek(&mut fixture, current_position, paused);
    assert_eq!(offset_target(&mut fixture, 0.0), current_position);
    (target, current_position + 5.0)
}

#[test]
fn paused_seek_keeps_offset_target_until_ack() {
    let (actual, expected) = offset_after_unacknowledged_seek(true);
    assert_eq!(actual, expected);
}

#[test]
fn acknowledged_playing_seek_offset_uses_fresh_progress() {
    let (mut fixture, current_position) = delayed_seek_fixture(false);
    acknowledge_current_seek(&mut fixture, current_position, false);
    assert_eq!(offset_target(&mut fixture, 5.0), current_position + 5.0);
    assert_eq!(offset_target(&mut fixture, 0.0), current_position);
}

#[test]
fn playing_seek_offset_uses_fresh_progress() {
    let (actual, expected) = offset_after_unacknowledged_seek(false);
    assert!(
        (actual - expected).abs() < 0.001,
        "an offset adjustment must preserve playback progress after the unacknowledged seek: actual={actual}, expected={expected}"
    );
}

#[test]
fn repeated_offset_changes_preserve_confirmed_pending_seek_progress() {
    let (mut fixture, current_position) = delayed_seek_fixture(false);
    assert_eq!(offset_target(&mut fixture, 5.0), current_position + 5.0);
    assert_eq!(offset_target(&mut fixture, -2.0), current_position - 2.0);
    assert_eq!(offset_target(&mut fixture, 0.0), current_position);
    acknowledge_current_seek(&mut fixture, current_position, false);
    assert_eq!(offset_target(&mut fixture, 5.0), current_position + 5.0);
}

#[test]
fn first_delayed_playing_sample_can_confirm_seek_progress() {
    let mut fixture = SeekFixture::with_paused(false);
    fixture.emit_seek();
    // The authoritative snapshot spans missing intermediate position events.
    fixture.sequence = 20;
    fixture.queue_observation(false, 13.0);
    fixture
        .runtime
        .drain_player_transport_coordination(3.0)
        .unwrap();
    assert_eq!(offset_target(&mut fixture, 5.0), 18.0);
    acknowledge_current_seek(&mut fixture, 13.0, false);
    assert_eq!(offset_target(&mut fixture, 0.0), 13.0);
}

#[test]
fn pending_seek_progress_requires_a_position_newer_than_the_pre_seek_sample() {
    for (observed_at_millis, position, expected_offset_target) in
        [(1100, 11.0, 16.0), (1200, 11.1, 16.2)]
    {
        let mut fixture = SeekFixture::with_paused(false);
        fixture.queue_observation_at(false, 11.0, 1100);
        fixture
            .runtime
            .drain_player_transport_coordination(1.1)
            .unwrap();
        fixture.emit_seek();

        // A new ordered delivery can still carry the pre-seek capture time.
        // Even when its position matches the target, it cannot confirm that
        // the physical seek has landed or authorize projected seek progress.
        fixture.queue_observation_at(false, position, observed_at_millis);
        fixture
            .runtime
            .drain_player_transport_coordination(1.2)
            .unwrap();
        fixture.runtime.player.commands.clear();
        fixture
            .runtime
            .set_local_playback_offset_seconds(5.0, 1.3)
            .unwrap();
        let Some(PlayerCommand::SetPosition(target)) = fixture.runtime.player.commands.last()
        else {
            panic!("changing the offset must issue a player position command");
        };
        assert!(
            (*target - expected_offset_target).abs() < 0.001,
            "sample at {observed_at_millis}ms must seek to {expected_offset_target}, got {target}"
        );

        // The next fresh physical sample confirms progress in the new offset
        // coordinates. Removing that offset must retain the observed progress.
        fixture.queue_observation_at(false, 16.3, 1400);
        fixture
            .runtime
            .drain_player_transport_coordination(1.4)
            .unwrap();
        fixture.runtime.player.commands.clear();
        fixture
            .runtime
            .set_local_playback_offset_seconds(0.0, 1.5)
            .unwrap();
        let Some(PlayerCommand::SetPosition(target)) = fixture.runtime.player.commands.last()
        else {
            panic!("removing the offset must issue a player position command");
        };
        assert!((*target - 11.4).abs() < 0.001);
    }
}

#[test]
fn predecessor_position_cannot_confirm_a_forward_or_backward_seek() {
    for predecessor_position in [0.0, 100.0] {
        let mut fixture = SeekFixture::with_paused(false);
        fixture.queue_observation(false, predecessor_position);
        fixture
            .runtime
            .drain_player_transport_coordination(1.1)
            .unwrap();
        fixture.emit_seek();
        fixture.queue_observation(false, predecessor_position + 0.1);
        fixture
            .runtime
            .drain_player_transport_coordination(1.2)
            .unwrap();
        fixture.runtime.player.commands.clear();
        fixture
            .runtime
            .set_local_playback_offset_seconds(5.0, 1.2)
            .unwrap();
        assert_eq!(
            fixture.runtime.player.commands.last(),
            Some(&PlayerCommand::SetPosition(16.0))
        );
        // The next actual target observation establishes ordinary progress.
        fixture.queue_observation(false, 16.2);
        fixture
            .runtime
            .drain_player_transport_coordination(1.3)
            .unwrap();
        fixture.runtime.player.commands.clear();
        fixture
            .runtime
            .set_local_playback_offset_seconds(0.0, 1.3)
            .unwrap();
        assert_eq!(
            fixture.runtime.player.commands.last(),
            Some(&PlayerCommand::SetPosition(11.2))
        );
    }
}

#[test]
fn room_change_discards_unacknowledged_offset_anchor() {
    let (mut fixture, _) = delayed_seek_fixture(false);
    assert!(fixture.runtime.run_set_room("room2").unwrap());
    fixture
        .runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room2"},"version":"1.7.5"}}"#,
            3.0,
        )
        .unwrap();
    fixture.runtime.session_mut().apply_message_json_at(
        r#"{"State":{"playstate":{"position":20.0,"paused":true,"doSeek":true,"setBy":"bob","sorotteTransportRevision":1}}}"#, 3.0).unwrap();
    assert_eq!(offset_target(&mut fixture, 5.0), 25.0);
    fixture.runtime.player.commands.clear();
    assert!(fixture.runtime.run_seek_to_position(15.0).unwrap());
    assert!(
        fixture
            .runtime
            .player
            .commands
            .contains(&PlayerCommand::SetPosition(20.0))
    );
}
