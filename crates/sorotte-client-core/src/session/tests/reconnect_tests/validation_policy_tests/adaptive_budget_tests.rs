use super::*;

#[test]
fn client_runtime_reconnect_state_restore_validation_adaptive_retry_backoff_scales_after_prior_exhaustion()
 {
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_backoff = true;
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.model.reconnect.state_restore_validation_pending = true;

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
        .expect("first correction failure should exhaust retry budget");
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
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 1,
                max_attempts: 0,
            },
        ]
    );
    assert_eq!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions,
        1
    );

    runtime
        .session_mut_for_test()
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 1;
    runtime
        .session_mut_for_test()
        .model
        .reconnect
        .state_restore_validation_pending = true;
    runtime.session_mut_for_test().model.playback.local_paused = Some(true);
    runtime.session_mut_for_test().model.playback.local_position = Some(117.5);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("adaptive retry backoff should schedule a retry in the next restore cycle");
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
                cooldown_ticks: 2,
            },
        ],
        "one prior retry-exhausted restore cycle should double the first retry cooldown when adaptive cycle backoff is enabled"
    );
    assert_eq!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_retry_cooldown_ticks,
        2
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_adaptive_retry_budget_reduces_after_prior_exhaustion()
 {
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.model.reconnect.state_restore_validation_pending = true;

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
        .expect("first correction failure should exhaust zero retry budget");
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
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 1,
                max_attempts: 0,
            },
        ]
    );
    assert_eq!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions,
        1
    );

    runtime
        .session_mut_for_test()
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 2;
    runtime
        .session_mut_for_test()
        .model
        .reconnect
        .state_restore_validation_pending = true;
    runtime.session_mut_for_test().model.playback.local_paused = Some(true);
    runtime.session_mut_for_test().model.playback.local_position = Some(117.5);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second restore cycle should use reduced adaptive retry budget");
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
        ],
        "one prior retry-exhausted restore cycle should reduce the effective retry budget by one attempt when adaptive budget is enabled"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure in reduced budget cycle should exhaust retries");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 2,
                max_attempts: 1,
            },
        ]
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_adaptive_retry_budget_honors_min_attempt_floor()
 {
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 3;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts = 2;
    session
        .model
        .reconnect
        .state_restore_correction_consecutive_retry_exhaustions = 5;
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.model.reconnect.state_restore_validation_pending = true;

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
        .expect("adaptive retry budget floor should still allow retries");
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
                max_attempts: 2,
                cooldown_ticks: 0,
            },
        ],
        "adaptive retry budget floor should cap reductions so the effective retry budget does not fall below the configured minimum"
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_success_resets_adaptive_retry_backoff_history()
{
    let mut session = ClientSession::default();
    session.model.room.name = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_backoff = true;
    session.model.room.playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.model.reconnect.state_restore_validation_pending = true;
    session
        .model
        .reconnect
        .state_restore_correction_consecutive_retry_exhaustions = 2;

    let player = RecordingPlayer {
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
        .expect("successful correction should complete reconnect validation");

    assert!(
        !runtime
            .session()
            .model
            .reconnect
            .state_restore_validation_pending
    );
    assert_eq!(runtime.player().position, Some(120.0));
    assert_eq!(
        runtime
            .session()
            .model
            .reconnect
            .state_restore_correction_consecutive_retry_exhaustions,
        0,
        "adaptive retry backoff history should reset after a successful correction"
    );
}
