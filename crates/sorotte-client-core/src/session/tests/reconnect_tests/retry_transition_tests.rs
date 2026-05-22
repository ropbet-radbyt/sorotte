use super::*;

#[test]
fn reconnect_retry_policy_uses_legacy_exponential_backoff_with_cap() {
    let mut session = ClientSession::default();

    let retry0 = session.plan_reconnect_retry(0);
    assert!(retry0.should_retry);
    assert_eq!(retry0.delay_seconds, Some(0.1));

    let retry1 = session.plan_reconnect_retry(1);
    assert!(retry1.should_retry);
    assert_eq!(retry1.delay_seconds, Some(0.2));

    let retry5 = session.plan_reconnect_retry(5);
    assert!(retry5.should_retry);
    assert_eq!(retry5.delay_seconds, Some(3.2));

    let retry8 = session.plan_reconnect_retry(8);
    assert!(retry8.should_retry);
    assert_eq!(retry8.delay_seconds, Some(3.2));
}

#[test]
fn reconnect_retry_policy_respects_max_retry_cutoff() {
    let mut session = ClientSession::default();
    session.reconnect_policy_mut().max_retries = 2;

    let retry = session.plan_reconnect_retry(3);
    assert!(!retry.should_retry);
    assert_eq!(retry.delay_seconds, None);
    assert!(retry.should_reset_state);
}

#[test]
fn reconnect_retry_policy_applies_sync_state_reset_before_retry() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

    let outbound =
        session.reconcile_state_and_build_response(StatePayload::new(), 0.0, true, 10.0, 0.1);
    assert!(
        outbound
            .ignoring_on_the_fly
            .as_ref()
            .is_some_and(|ignore| ignore.client == Some(1)),
        "precondition: outbound state should carry client ignore counter"
    );

    let retry = session.plan_reconnect_retry(0);
    assert!(retry.should_retry);
    assert_eq!(retry.delay_seconds, Some(0.1));
    assert!(retry.should_reset_state);
    assert_eq!(session.client_ignoring_on_the_fly(), 0);
    assert_eq!(session.server_ignoring_on_the_fly(), 0);
    assert!(session.current_room_playstate().is_none());
}

#[test]
fn runtime_actions_for_reconnect_retry_schedule_or_stop_as_expected() {
    let mut session = ClientSession::default();

    let retry_actions = session.runtime_actions_for_reconnect_retry(0);
    assert_eq!(
        retry_actions,
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Attempting {
                    retries: 0,
                    delay_seconds: 0.1,
                },
            ),
            ClientRuntimeAction::ScheduleReconnect { delay_seconds: 0.1 },
        ]
    );

    session.reconnect_policy_mut().max_retries = 0;
    let stop_actions = session.runtime_actions_for_reconnect_retry(1);
    assert_eq!(
        stop_actions,
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Disconnected,
            ),
            ClientRuntimeAction::StopReconnect,
        ]
    );
}

#[test]
fn reconnect_transition_connected_notification_emits_after_reconnect_hello() {
    let mut session = ClientSession::default();
    session.runtime_actions_for_reconnect_retry(0);
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    assert_eq!(
        session.runtime_actions_for_reconnect_transition_if_needed(),
        vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Connected,
        )]
    );
    assert!(
        session
            .runtime_actions_for_reconnect_transition_if_needed()
            .is_empty(),
        "connected reconnect notification should drain once"
    );
}

#[test]
fn client_runtime_reconnect_retry_and_recovery_notifications_preserve_sequence_without_noop_duplicates()
 {
    let mut session = ClientSession::default();
    session.room = Some("room1".to_owned());
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
        .expect("first correction failure should schedule retry");
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
                cooldown_ticks: 1,
            },
        ],
        "first failure should emit mismatch then retry-scheduled"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("cooldown tick should defer retry");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "cooldown tick should not duplicate reconnect notifications"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("second failure should exhaust retry budget and activate recovery cooldown");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                attempts: 2,
                max_attempts: 1,
            },
        ],
        "retry exhaustion should emit only give-up notification without repeating mismatch details"
    );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "retry exhaustion should clear pending validation"
    );
    assert_eq!(
        runtime
            .session()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
        1
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("repeated no-op tick after exhaustion should not emit notifications");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op validation tick after exhaustion should not duplicate reconnect notifications"
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
            "suppressed recovery cycle should emit mismatch then suppression notification"
        );
    assert!(
        !runtime.session().reconnect_state_restore_validation_pending,
        "suppressed cycle should clear pending validation"
    );

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("repeated no-op tick after suppressed cycle should not emit notifications");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op validation tick after suppressed cycle should not duplicate suppression notifications"
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
        .expect("correction should re-enable after recovery cooldown");
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
            ],
            "reenabled cycle should emit reenabled notification before mismatch details"
        );
    assert_eq!(runtime.player().position, Some(140.0));

    runtime
        .run_reconnect_state_restore_validation_if_needed()
        .expect("repeated no-op tick after reenabled success should not emit notifications");
    assert!(
        runtime.drain_reconnect_notifications().is_empty(),
        "no-op validation tick after reenabled correction should not duplicate reenabled/mismatch notifications"
    );
}

#[test]
fn client_runtime_flush_helpers_expose_protocol_and_reconnect_intents() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_readiness_unpause_attempt(10.0, true, true, false)
        .expect("readiness attempt should dispatch");
    let outbound_lines = runtime
        .flush_queued_protocol_lines()
        .expect("queued outbound lines should encode");
    assert_eq!(outbound_lines.len(), 1);
    assert!(outbound_lines[0].contains("\"isReady\":true"));

    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry should dispatch");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![ReconnectTransitionNotification::Attempting {
            retries: 0,
            delay_seconds: 0.1,
        }]
    );
    assert!(runtime.drain_reconnect_notifications().is_empty());
    assert_eq!(runtime.drain_reconnect_requests(), vec![0.1]);
    assert!(runtime.drain_reconnect_requests().is_empty());

    runtime.session_mut().reconnect_policy_mut().max_retries = 0;
    runtime
        .run_reconnect_retry(1)
        .expect("terminal reconnect retry should dispatch");
    assert_eq!(
        runtime.drain_reconnect_notifications(),
        vec![ReconnectTransitionNotification::Disconnected]
    );
    assert!(runtime.take_stop_reconnect_requested());
    assert!(!runtime.take_stop_reconnect_requested());
}

#[test]
fn client_runtime_drain_reconnect_intents_dispatches_scheduler_callbacks() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_retry(0)
        .expect("retry dispatch should queue reconnect delay");
    runtime.session_mut().reconnect_policy_mut().max_retries = 0;
    runtime
        .run_reconnect_retry(1)
        .expect("terminal retry should queue stop intent");

    let mut scheduled = Vec::new();
    let mut stop_calls = 0_usize;
    runtime.drain_reconnect_intents(
        |delay_seconds| scheduled.push(delay_seconds),
        || stop_calls += 1,
    );

    assert_eq!(scheduled, vec![0.1]);
    assert_eq!(stop_calls, 1);

    runtime.drain_reconnect_intents(
        |delay_seconds| scheduled.push(delay_seconds),
        || stop_calls += 1,
    );
    assert_eq!(scheduled, vec![0.1]);
    assert_eq!(stop_calls, 1);
}

#[test]
fn client_runtime_run_reconnect_transition_dispatches_connected_notification() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry should dispatch");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
        .run_reconnect_transition_if_needed()
        .expect("reconnect transition dispatch should succeed");

    assert_eq!(
        runtime.control().reconnect_notifications(),
        &[
            ReconnectTransitionNotification::Attempting {
                retries: 0,
                delay_seconds: 0.1,
            },
            ReconnectTransitionNotification::Connected,
        ]
    );
}

#[test]
fn client_runtime_drain_reconnect_notifications_to_sink_dispatches_callback() {
    let session = ClientSession::default();
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry should dispatch");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
        .run_reconnect_transition_if_needed()
        .expect("reconnect transition dispatch should succeed");

    let mut captured = Vec::new();
    runtime
        .drain_reconnect_notifications_to_sink(|notification| {
            captured.push(notification.clone());
            Ok::<(), ()>(())
        })
        .expect("reconnect notification sink dispatch should succeed");

    assert_eq!(
        captured,
        vec![
            ReconnectTransitionNotification::Attempting {
                retries: 0,
                delay_seconds: 0.1,
            },
            ReconnectTransitionNotification::Connected,
        ]
    );
    assert!(runtime.drain_reconnect_notifications().is_empty());
}
