use super::*;

#[test]
fn client_runtime_reconnect_state_restore_validation_honors_retry_budget() {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;

    let player = RecordingPlayer {
        fail_set_position: true,
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first correction failure should schedule retry within configured budget");
    assert!(runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_attempts,
        1
    );
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            },
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: 1,
                max_attempts: 1,
                cooldown_ticks: 0,
            },
        ]
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure should give up after configured retry budget");

    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "retry budget should clear pending validation after a repeated correction failure"
    );
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 2,
                max_attempts: 1,
            }
        ],
        "retry exhaustion should emit a give-up notification without duplicating mismatch details"
    );
    assert_eq!(
        runtime.player().position,
        None,
        "failed correction should not claim success before retry budget is exhausted"
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_honors_custom_retry_cooldown() {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 2;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 2;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;

    let player = RecordingPlayer {
        fail_set_position: true,
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first correction failure should be swallowed");
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        2
    );
    runtime.player_mut().fail_set_position = false;

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first cooldown tick should defer retry");
    assert_eq!(runtime.player().position, None);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        1
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second cooldown tick should still defer retry");
    assert_eq!(runtime.player().position, None);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        0
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("retry should run after configured cooldown expires");
    assert_eq!(runtime.player().position, Some(120.0));
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
}

#[test]
fn client_runtime_reconnect_state_restore_validation_honors_exponential_retry_backoff_cap() {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 3;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_exponential_backoff = true;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_cooldown_ticks = 2;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;

    let player = RecordingPlayer {
        fail_set_position: true,
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first failure should schedule retry with base cooldown");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            },
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: 1,
                max_attempts: 3,
                cooldown_ticks: 1,
            },
        ]
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        1
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("cooldown tick should defer second retry attempt");
    assert!(runtime.drain_reconnect_notifications().is_empty());
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        0
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure should use exponential cooldown");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: 2,
                max_attempts: 3,
                cooldown_ticks: 2,
            },
        ]
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        2
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first cooldown tick after second failure should defer retry");
    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second cooldown tick after second failure should defer retry");
    assert!(runtime.drain_reconnect_notifications().is_empty());
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        0
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("third failure should apply capped exponential cooldown");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: 3,
                max_attempts: 3,
                cooldown_ticks: 2,
            },
        ],
        "third exponential cooldown would be 4 ticks but should clamp to configured max"
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_cooldown_ticks,
        2
    );
}
