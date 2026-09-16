use super::*;

fn playing_runtime() -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    playing_runtime_with_player(false)
}

fn playing_runtime_with_player(
    managed: bool,
) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut session = readiness_v2_session(1, 41, 0);
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    session.model.playback.local_paused = Some(false);
    let mut runtime = ClientRuntime::new(
        session,
        CoordinatedTestPlayer {
            advertises_telemetry: managed,
            ..Default::default()
        },
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("native-pause-race").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    if managed {
        runtime
            .player_mut_for_test()
            .queue_from_started_media(transport(1, 0.0, PlayerTransportPhase::Playing, 1.0));
        runtime.drain_player_transport_coordination(0.0).unwrap();
        runtime.player_mut_for_test().commands.clear();
    } else {
        runtime.observe_external_player_transport(
            transport(1, 0.0, PlayerTransportPhase::Playing, 1.0),
            0.0,
        );
        runtime.reconcile_external_player_playback(0.0);
    }
    runtime.deliver_queued_protocol_messages();
    runtime
}

fn is_unpause(action: &PlaybackCoordinatorAction) -> bool {
    matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(false) | CoordinatorPlayerCommand::Play(_),
            ..
        }
    )
}

#[test]
fn native_pause_during_healthy_recovery_is_confirmed_without_resuming_the_player() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Rebuffering, 1.1),
        0.1,
    );
    for now in [0.2, 0.4] {
        runtime.observe_external_player_transport(
            transport(1, now, PlayerTransportPhase::Playing, 1.0 + now),
            now,
        );
    }
    assert!(
        runtime
            .playback_coordination
            .coordinator
            .recovery_episode()
            .is_some()
    );
    for (now, phase) in [
        (0.5, PlayerTransportPhase::Playing),
        (0.7, PlayerTransportPhase::ReadyPaused),
    ] {
        let actions =
            runtime.observe_external_player_transport(paused_transport(1, now, phase, 1.5), now);
        assert!(
            actions.iter().all(|action| !is_unpause(action)),
            "healthy recovery must not erase a native pause candidate: {actions:?}"
        );
    }
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::NotReady)
    );
}

#[test]
fn pause_during_an_unresolved_recovery_seek_remains_technical() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Rebuffering, 1.1),
        0.1,
    );
    runtime.observe_external_player_transport(
        transport(1, 10.0, PlayerTransportPhase::Playing, 1.2),
        10.0,
    );
    let recovery_actions = runtime.observe_external_player_transport(
        transport(1, 10.1, PlayerTransportPhase::Playing, 1.3),
        10.1,
    );
    assert!(
        recovery_actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )),
        "fixture must leave a recovery seek in flight: {recovery_actions:?}"
    );
    for now in [10.2, 10.4] {
        runtime.observe_external_player_transport(
            paused_transport(1, now, PlayerTransportPhase::ReadyPaused, 1.3),
            now,
        );
    }
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(
        runtime
            .playback_coordination
            .pending_native_pause_authority_fence
            .is_none()
    );
}

#[test]
fn recovery_command_pause_cannot_become_native_intent() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        transport(1, 0.1, PlayerTransportPhase::Rebuffering, 1.1),
        0.1,
    );
    runtime.observe_external_player_transport(
        transport(1, 0.2, PlayerTransportPhase::Playing, 1.2),
        0.2,
    );
    runtime
        .playback_coordination
        .register_external_pause_command_result(PlayerCommandCause::Recovery, true, true, 0.25);
    for now in [0.3, 0.5] {
        runtime.observe_external_player_transport(
            paused_transport(1, now, PlayerTransportPhase::ReadyPaused, 1.3),
            now,
        );
    }
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(
        runtime
            .playback_coordination
            .pending_native_pause_authority_fence
            .is_none()
    );
}

#[test]
fn native_pause_survives_reconciliation_until_stable_observation() {
    let mut runtime = playing_runtime();
    let first = runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    assert!(
        first.iter().all(|action| !is_unpause(action)),
        "reconciliation must not erase an unowned pause before the classifier can confirm it: {first:?}"
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    let between = runtime.reconcile_external_player_playback(0.15);
    assert!(between.iter().all(|action| !is_unpause(action)));
    let confirmed = runtime.observe_external_player_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
    );
    assert!(confirmed.iter().all(|action| !is_unpause(action)));
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::NotReady)
    );
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(true)
    );
}

#[test]
fn native_pause_hold_expires_without_fabricating_a_confirming_observation() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    let expired = runtime.reconcile_external_player_playback(1.2);
    assert!(
        expired.iter().any(is_unpause),
        "unconfirmed pause must not block room authority indefinitely"
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    runtime.observe_external_player_transport(
        paused_transport(1, 1.3, PlayerTransportPhase::ReadyPaused, 1.1),
        1.3,
    );
    assert!(
        runtime.session().pending_readiness_intent().is_none(),
        "expired edge must not be resurrected"
    );
}

#[test]
fn newer_room_play_supersedes_unconfirmed_native_pause() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    runtime.session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.2,"paused":false,"doSeek":false,"setBy":"charlie"}}}"#,
        0.15,
    ).unwrap();
    let newer_authority = runtime.reconcile_external_player_playback(0.15);
    assert!(newer_authority.iter().any(is_unpause));
    runtime.observe_external_player_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
    );
    assert!(
        runtime.session().pending_readiness_intent().is_none(),
        "old candidate must not acquire new room authority"
    );
}

#[test]
fn same_revision_room_heartbeat_does_not_cancel_native_pause_confirmation() {
    let mut runtime = playing_runtime();
    runtime.session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.02,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":7}}}"#,
        0.02,
    ).unwrap();
    runtime.reconcile_external_player_playback(0.02);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    runtime.session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.15,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":7}}}"#,
        0.15,
    ).unwrap();
    let heartbeat = runtime.reconcile_external_player_playback(0.15);
    assert!(
        heartbeat.iter().all(|action| !is_unpause(action)),
        "same-revision liveness is not a new transport decision: {heartbeat:?}"
    );
    runtime.observe_external_player_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
    );
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::NotReady)
    );
}

#[test]
fn newer_tagged_transport_revision_cancels_native_pause_confirmation() {
    let mut runtime = playing_runtime();
    runtime.session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.02,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":7}}}"#,
        0.02,
    ).unwrap();
    runtime.reconcile_external_player_playback(0.02);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    runtime.session.apply_message_json_at(
        r#"{"State":{"playstate":{"position":1.15,"paused":false,"doSeek":false,"setBy":"bob","sorotteTransportRevision":8}}}"#,
        0.15,
    ).unwrap();
    let actions = runtime.reconcile_external_player_playback(0.15);
    assert!(actions.iter().any(is_unpause));
    runtime.observe_external_player_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
}

#[test]
fn transient_pause_does_not_publish_not_ready_and_a_later_pause_still_works() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    runtime.observe_external_player_transport(
        transport(1, 0.14, PlayerTransportPhase::Playing, 1.1),
        0.14,
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    for now in [0.5, 0.7] {
        let actions = runtime.observe_external_player_transport(
            paused_transport(1, now, PlayerTransportPhase::ReadyPaused, 1.5),
            now,
        );
        assert!(actions.iter().all(|action| !is_unpause(action)));
    }
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::NotReady)
    );
}

#[test]
fn command_owned_pause_does_not_withhold_authoritative_unpause() {
    let mut runtime = playing_runtime();
    runtime
        .playback_coordination
        .register_external_pause_command_result(
            PlayerCommandCause::RemoteRoomSynchronization,
            true,
            true,
            0.05,
        );
    let actions = runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    assert!(actions.iter().any(is_unpause));
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::OwnedCommand { .. })
    ));
}

#[test]
fn cache_pause_never_acquires_native_pause_authority() {
    let mut runtime = playing_runtime();
    let mut update = paused_transport(1, 0.1, PlayerTransportPhase::Rebuffering, 1.1);
    update.paused_for_cache = Some(true);
    runtime.observe_external_player_transport(update, 0.1);
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(
        runtime
            .playback_coordination
            .pending_native_pause_authority_fence
            .is_none()
    );
    assert!(matches!(
        runtime
            .playback_coordination
            .last_player_transition_classification,
        Some(PlayerTransitionClassification::Technical {
            action: NativePlayerAction::Pause,
            ..
        })
    ));
}

#[test]
fn managed_ordered_player_pause_uses_the_same_stability_hold() {
    let mut runtime = playing_runtime_with_player(true);
    for now in [0.1, 0.3] {
        runtime
            .player_mut_for_test()
            .queue_from_started_media(paused_transport(
                1,
                now,
                PlayerTransportPhase::ReadyPaused,
                1.1,
            ));
        runtime.drain_player_transport_coordination(now).unwrap();
        assert!(
            runtime.player().commands.iter().all(|command| !matches!(
                command,
                PlayerCommand::SetPaused(false) | PlayerCommand::Play(_)
            )),
            "managed player must not undo its native pause: {:?}",
            runtime.player().commands
        );
        if now == 0.1 {
            assert!(runtime.session().pending_readiness_intent().is_none());
        }
    }
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::NotReady)
    );
}

#[test]
fn adapter_replacement_retires_pending_pause_and_stale_samples_cannot_confirm_it() {
    let mut runtime = playing_runtime();
    let old_epoch = runtime.playback_transport_adapter_epoch();
    runtime.observe_external_player_transport_at_epoch(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
        old_epoch,
    );
    runtime.reset_playback_transport_adapter_epoch(0.15);
    runtime.observe_external_player_transport_at_epoch(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
        old_epoch,
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(
        runtime
            .playback_coordination
            .pending_native_pause_authority_fence
            .is_none()
    );
}

#[test]
fn pending_native_pause_cannot_cross_a_connection_generation() {
    let mut runtime = playing_runtime();
    runtime.observe_external_player_transport(
        paused_transport(1, 0.1, PlayerTransportPhase::ReadyPaused, 1.1),
        0.1,
    );
    runtime
        .playback_coordination
        .begin_protocol_connection_generation(&runtime.session);
    runtime.observe_external_player_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 1.1),
        0.3,
    );
    assert!(runtime.session().pending_readiness_intent().is_none());
    assert!(
        runtime
            .playback_coordination
            .pending_native_pause_authority_fence
            .is_none()
    );
}
