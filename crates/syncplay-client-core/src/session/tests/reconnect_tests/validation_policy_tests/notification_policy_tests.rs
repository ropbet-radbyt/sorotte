use super::*;

#[test]
fn client_runtime_reconnect_warn_only_on_exhaustion_retry_and_recovery_notifications_follow_policy_specific_sequence()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;
    session.local_paused = Some(true);
    session.local_position = Some(117.5);

    let player = RecordingPlayer {
        fail_set_position: true,
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect(
            "warn-only-on-exhaustion should attempt correction but suppress early notifications",
        );
    assert!(
        runtime.session().reconnect_state_restore_validation_pending,
        "first failure should leave validation pending for retry"
    );
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "warn-only-on-exhaustion should suppress mismatch and retry-scheduled notifications"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("cooldown tick should defer retry");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "cooldown no-op ticks should not emit notifications"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure should exhaust retry budget");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 2,
                max_attempts: 1,
            },
        ],
        "warn-only-on-exhaustion should emit only retries-exhausted on give-up"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "retry exhaustion should clear pending validation"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("post-exhaustion no-op tick should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "post-exhaustion no-op ticks should not duplicate exhaustion warnings"
    );

    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(130.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime.session_mut().local_paused = Some(true);
    runtime.session_mut().local_position = Some(125.0);
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime
        .session_mut()
        .begin_reconnect_state_restore_validation_cycle();

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("recovery cooldown cycle should suppress correction");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "warn-only-on-exhaustion should suppress mismatch visibility during recovery cooldown and emit only suppression notice"
        );
    assert_eq!(runtime.player().position, None);
    assert!(!runtime.session().reconnect_state_restore_validation_pending);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("post-suppression no-op tick should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "post-suppression no-op ticks should not duplicate suppression notices"
    );

    runtime.player_mut().fail_set_position = false;
    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(140.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime.session_mut().local_paused = Some(true);
    runtime.session_mut().local_position = Some(135.0);
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime
        .session_mut()
        .begin_reconnect_state_restore_validation_cycle();

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("recovery cooldown re-enable cycle should correct");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
            ],
            "warn-only-on-exhaustion should emit only reenabled notification (no mismatch detail) on the recovery re-enable cycle"
        );
    assert_eq!(runtime.player().position, Some(140.0));
    assert!(!runtime.session().reconnect_state_restore_validation_pending);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("post-success no-op tick should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "post-success no-op ticks should not duplicate reenabled notifications"
    );
}

#[test]
fn client_runtime_reconnect_notify_only_policy_keeps_retry_and_recovery_notifications_suppressed_across_cycles()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;
    session.local_paused = Some(true);
    session.local_position = Some(117.5);

    let player = RecordingPlayer {
        fail_set_position: true,
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("notify-only validation should emit mismatch without correction");
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
        ],
        "notify-only policy should emit only mismatch details (no retry scheduling)"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "notify-only validation should complete in one tick"
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        0,
        "notify-only policy should not activate recovery cooldown state"
    );
    assert_eq!(
        runtime.player().position,
        None,
        "notify-only policy should not attempt corrective seeks even if correction would fail"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("repeated no-op tick should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op validation ticks should not synthesize retry/recovery notifications in notify-only mode"
    );

    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(130.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime.session_mut().local_paused = Some(true);
    runtime.session_mut().local_position = Some(125.0);
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime
        .session_mut()
        .begin_reconnect_state_restore_validation_cycle();

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("notify-only should stay mismatch-only across reconnect validation cycles");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 125.0,
                room_position: 130.0,
                position_diff_seconds: 5.0,
            },
        ],
        "notify-only policy should remain mismatch-only on later reconnect validation cycles"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "notify-only validation should clear pending state on later cycles too"
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        0,
        "notify-only policy should keep recovery cooldown disabled across cycles"
    );
    assert_eq!(
        runtime.player().position,
        None,
        "notify-only policy should not perform corrective seeks on later cycles"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("post-cycle no-op tick should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "notify-only policy should not emit retry/recovery notifications on repeated no-op ticks across cycles"
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_notify_only_mode_skips_correction() {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_auto_correct = false;
    session.room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(120.0),
            paused: Some(false),
            ..RoomPlaystateView::default()
        },
    );
    session.reconnect_state_restore_validation_pending = true;

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
        .expect("notify-only validation should not fail");

    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: false,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            }
        ]
    );
    assert_eq!(runtime.player().paused, None);
    assert_eq!(runtime.player().position, None);
    assert_eq!(
        runtime.session().local_paused,
        Some(true),
        "notify-only mode should leave telemetry-refreshed local state unchanged"
    );
    assert_eq!(runtime.session().local_position, Some(117.5));
}

#[test]
fn client_runtime_reconnect_state_restore_validation_warn_only_on_exhaustion_suppresses_early_notifications()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion);
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
        .expect("warn-only-on-exhaustion policy should still attempt correction");

    assert!(runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_validation_retry_attempts,
        1
    );
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "warn-only-on-exhaustion should suppress mismatch and retry notifications before exhaustion"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure should exhaust retry budget and emit a single warning");

    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 2,
                max_attempts: 1,
            }
        ]
    );
}
