use super::*;

#[test]
fn client_runtime_reconnect_disable_after_n_mismatches_notifications_follow_sequence_without_noop_duplicates()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
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

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first mismatch cycle should auto-correct before disable threshold");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            }
        ],
        "first mismatch cycle should emit mismatch details only"
    );
    assert_eq!(runtime.player().position, Some(120.0));
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "first cycle should complete validation after correction"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("no-op tick after first cycle should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op tick after first cycle should not duplicate mismatch notifications"
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
        .expect("threshold-reaching cycle should disable correction");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles: 2,
                disable_after_mismatch_cycles: 2,
            }],
            "threshold cycle should emit only disable-after-repeated-mismatches notification"
        );
    assert_eq!(
        runtime.player().position,
        Some(120.0),
        "disable threshold cycle should not issue a corrective seek"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "threshold cycle should clear pending validation"
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        1,
        "disable threshold cycle should activate recovery cooldown"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("no-op tick after disable notification should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op tick after disable notification should not duplicate notifications"
    );

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
        .expect("recovery cooldown cycle should suppress correction once");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "suppressed recovery cycle should emit mismatch then recovery-cooldown-suppressed"
        );
    assert_eq!(
        runtime.player().position,
        Some(120.0),
        "suppressed recovery cycle should not issue a corrective seek"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "suppressed recovery cycle should clear pending validation"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("no-op tick after suppressed recovery cycle should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op tick after suppressed recovery cycle should not duplicate suppression notifications"
    );

    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(150.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime.session_mut().local_paused = Some(true);
    runtime.session_mut().local_position = Some(145.0);
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime
        .session_mut()
        .begin_reconnect_state_restore_validation_cycle();

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("re-enabled cycle should resume correction after cooldown");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 145.0,
                    room_position: 150.0,
                    position_diff_seconds: 5.0,
                },
            ],
            "re-enabled cycle should emit recovery-cooldown-reenabled before mismatch details"
        );
    assert_eq!(runtime.player().position, Some(150.0));
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "re-enabled correction cycle should clear pending validation"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("no-op tick after re-enabled cycle should remain silent");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op tick after re-enabled cycle should not duplicate reenabled or mismatch notifications"
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_recovery_cooldown_suppresses_cycle_after_retry_exhaustion_then_reenables()
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
        .expect("first reconnect correction failure should exhaust retries");
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
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        1
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
        .expect("recovery cooldown should suppress correction for one reconnect cycle");
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
            ],
            "suppressed recovery cycle should emit mismatch visibility but skip corrective actions"
        );
    assert_eq!(runtime.player().position, None);
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        0
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
        .expect("correction should re-enable after recovery cooldown cycle completes");
    assert_eq!(runtime.player().position, Some(140.0));
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
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
}

#[test]
fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_stops_correction_on_threshold()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
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
        .expect("first mismatch cycle should still auto-correct before threshold");
    assert_eq!(runtime.player().position, Some(120.0));
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            }
        ]
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        1
    );

    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(130.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime.player_mut().pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_paused(true)
            .with_position_seconds(125.0),
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect(
            "threshold-reaching mismatch cycle should disable correction instead of correcting",
        );

    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles: 2,
                    disable_after_mismatch_cycles: 2,
                }
            ]
        );
    assert_eq!(
        runtime.player().position,
        Some(120.0),
        "no corrective seek should be issued once disable-after-N-mismatches threshold is reached"
    );
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        2
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_decays_counter_after_successful_correction_when_configured()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_disable_after_mismatch_decay_on_success = 1;
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

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first restore mismatch should be corrected");
    assert_eq!(
        runtime.player().position,
        Some(120.0),
        "first reconnect mismatch should still auto-correct"
    );
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 117.5,
                room_position: 120.0,
                position_diff_seconds: 2.5,
            }
        ]
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        0,
        "configured decay-on-success should recover the repeated-mismatch counter after successful correction"
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
        .run_reconnect_state_restore_validation_if_needed()
        .expect(
            "second restore mismatch should still correct instead of hitting disable threshold",
        );
    assert_eq!(
        runtime.player().position,
        Some(130.0),
        "decay-on-success should prevent threshold accumulation across successful correction cycles"
    );
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: true,
                room_paused: true,
                local_position: 125.0,
                room_position: 130.0,
                position_diff_seconds: 5.0,
            }
        ]
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        0
    );
}

#[test]
fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_recovery_cooldown_reenables_correction()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_policy_mode_override =
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
    session
        .behavior_config_mut()
        .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
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

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("first mismatch should correct and increment mismatch-cycle counter");
    assert_eq!(runtime.player().position, Some(120.0));
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
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
            }
        ]
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
        .expect("second mismatch should trigger disable-after-N and start recovery cooldown");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles: 2,
                disable_after_mismatch_cycles: 2,
            }]
        );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        1
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        0,
        "disable path should reset mismatch-cycle counter when recovery cooldown is activated"
    );
    assert_eq!(runtime.player().position, Some(120.0));

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
        .expect("recovery cooldown cycle should suppress correction");
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ]
        );
    assert_eq!(runtime.player().position, Some(120.0));
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        0
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_consecutive_mismatch_cycles,
        0
    );

    runtime.session_mut().room_playstates.insert(
        "room1".to_owned(),
        RoomPlaystateView {
            position: Some(150.0),
            paused: Some(true),
            ..RoomPlaystateView::default()
        },
    );
    runtime.session_mut().local_paused = Some(true);
    runtime.session_mut().local_position = Some(145.0);
    runtime
        .session_mut()
        .reconnect_state_restore_validation_pending = true;
    runtime
        .session_mut()
        .begin_reconnect_state_restore_validation_cycle();

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("correction should re-enable after recovery cooldown cycle");
    assert_eq!(runtime.player().position, Some(150.0));
    assert!(!runtime.session().reconnect_state_restore_validation_pending);
    assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 145.0,
                    room_position: 150.0,
                    position_diff_seconds: 5.0,
                },
            ]
        );
}
