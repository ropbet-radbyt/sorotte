use super::*;

#[test]
fn reconnect_state_restore_correction_state_snapshot_reports_effective_policy_and_retry_budget() {
    let mut session = ClientSession::default();
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
    session
        .behavior_config_mut()
        .reconnect_state_restore_position_tolerance_seconds = -5.0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 5;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts = 2;
    session.reconnect_state_restore_validation_pending = true;
    session.reconnect_state_restore_validation_retry_attempts = 2;
    session.reconnect_state_restore_validation_retry_cooldown_ticks = 4;
    session.reconnect_state_restore_validation_mismatch_notified = true;
    session.reconnect_state_restore_validation_mismatch_seen_in_cycle = true;
    session.reconnect_state_restore_correction_consecutive_mismatch_cycles = 3;
    session.reconnect_state_restore_correction_consecutive_retry_exhaustions = 4;
    session.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 2;
    session.reconnect_state_restore_correction_recovery_suppressed_this_cycle = true;
    session.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;

    let snapshot = session.reconnect_state_restore_correction_state_snapshot();
    assert!(snapshot.validation_pending);
    assert_eq!(snapshot.retry_attempts, 2);
    assert_eq!(snapshot.retry_cooldown_ticks, 4);
    assert!(snapshot.mismatch_notified_in_cycle);
    assert!(snapshot.mismatch_seen_in_cycle);
    assert_eq!(
        snapshot.effective_policy_mode,
        ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches
    );
    assert_eq!(
        snapshot.position_tolerance_seconds, SEEK_THRESHOLD_SECONDS,
        "invalid tolerance should normalize to the default seek threshold in snapshots"
    );
    assert_eq!(
        snapshot.effective_retry_max_attempts, 2,
        "adaptive retry budget snapshot should respect the configured minimum floor"
    );
    assert_eq!(snapshot.consecutive_mismatch_cycles, 3);
    assert_eq!(snapshot.consecutive_retry_exhaustions, 4);
    assert_eq!(snapshot.recovery_cooldown_reconnect_cycles_remaining, 2);
    assert!(snapshot.correction_suppressed_for_recovery_cycle);
    assert!(!snapshot.correction_reenabled_for_recovery_cycle);

    assert_eq!(
        *session.reconnect_state_restore_correction_metrics(),
        ReconnectStateRestoreCorrectionMetrics::default(),
        "metrics should start at zero until reconnect validation cycles execute"
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_metrics_and_state_snapshot_track_retry_and_recovery_progress()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_max_attempts = 0;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
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
    session.local_paused = Some(true);
    session.local_position = Some(117.5);
    session.reconnect_state_restore_validation_pending = true;
    session.begin_reconnect_state_restore_validation_cycle();

    let player = RecordingPlayer {
        fail_set_position: true,
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first failure should exhaust zero retry budget and start recovery cooldown");
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
    let snapshot_after_exhaustion = runtime.reconnect_state_restore_correction_state_snapshot();
    assert!(!snapshot_after_exhaustion.validation_pending);
    assert_eq!(
        snapshot_after_exhaustion.recovery_cooldown_reconnect_cycles_remaining,
        1
    );
    assert_eq!(snapshot_after_exhaustion.consecutive_retry_exhaustions, 1);

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
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ]
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
        .expect("correction should re-enable and succeed after recovery cooldown");
    assert_eq!(runtime.player().position, Some(140.0));
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
            ]
        );

    let metrics = *runtime.reconnect_state_restore_correction_metrics();
    assert_eq!(metrics.validation_cycles_started, 3);
    assert_eq!(metrics.validation_cycles_completed_without_mismatch, 0);
    assert_eq!(
        metrics.validation_cycles_completed_with_successful_correction,
        1
    );
    assert_eq!(metrics.mismatch_cycles_detected, 2);
    assert_eq!(metrics.mismatch_notifications_emitted, 3);
    assert_eq!(metrics.correction_actions_attempted, 2);
    assert_eq!(metrics.correction_actions_succeeded, 1);
    assert_eq!(metrics.correction_action_failures, 1);
    assert_eq!(metrics.correction_retries_scheduled, 0);
    assert_eq!(metrics.correction_retry_exhaustions, 1);
    assert_eq!(metrics.correction_disables_after_repeated_mismatches, 0);
    assert_eq!(metrics.correction_recovery_cooldown_suppressed_cycles, 1);
    assert_eq!(metrics.correction_recovery_cooldown_reenabled_cycles, 1);

    let final_snapshot = runtime.reconnect_state_restore_correction_state_snapshot();
    assert!(!final_snapshot.validation_pending);
    assert_eq!(final_snapshot.retry_attempts, 0);
    assert_eq!(final_snapshot.retry_cooldown_ticks, 0);
    assert_eq!(final_snapshot.consecutive_retry_exhaustions, 0);
    assert_eq!(
        final_snapshot.recovery_cooldown_reconnect_cycles_remaining,
        0
    );
    assert!(!final_snapshot.correction_suppressed_for_recovery_cycle);
    assert!(!final_snapshot.correction_reenabled_for_recovery_cycle);
}
