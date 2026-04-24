use super::*;
use serde_json::json;

#[test]
fn reconnect_correction_metrics_delta_message_formats_changed_counters() {
    let previous = ReconnectStateRestoreCorrectionMetrics {
        validation_cycles_started: 1,
        mismatch_cycles_detected: 1,
        correction_actions_attempted: 2,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let current = ReconnectStateRestoreCorrectionMetrics {
        validation_cycles_started: 3,
        mismatch_cycles_detected: 2,
        correction_actions_attempted: 5,
        correction_action_failures: 1,
        correction_retry_exhaustions: 1,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };

    let message = reconnect_correction_metrics_delta_message(Some(&previous), &current)
        .expect("changed counters should produce a metrics delta message");
    assert!(message.starts_with("reconnect correction metrics: "));
    assert!(message.contains("cycles_started=+2 (total=3)"));
    assert!(message.contains("mismatch_cycles=+1 (total=2)"));
    assert!(message.contains("actions_attempted=+3 (total=5)"));
    assert!(message.contains("actions_failed=+1 (total=1)"));
    assert!(message.contains("retry_exhaustions=+1 (total=1)"));
    assert_eq!(
        reconnect_correction_metrics_delta_message(Some(&current), &current),
        None,
        "unchanged metrics should not emit duplicate diagnostics"
    );
}

#[test]
fn reconnect_correction_metrics_delta_message_localized_legacy_compatible_localizes_prefix() {
    let previous = ReconnectStateRestoreCorrectionMetrics {
        correction_actions_attempted: 1,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let current = ReconnectStateRestoreCorrectionMetrics {
        correction_actions_attempted: 3,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };

    let message = crate::reconnect_correction_metrics_delta_message_localized_legacy_compatible(
        Some(&previous),
        &current,
        Some("pt_BR"),
    )
    .expect("changed counters should produce a metrics delta message");
    assert!(message.starts_with("Metricas de correcao de reconexao: "));
    assert!(message.contains("actions_attempted=+2 (total=3)"));
}

#[test]
fn reconnect_correction_metrics_delta_json_line_formats_changed_counters() {
    let previous = ReconnectStateRestoreCorrectionMetrics {
        validation_cycles_started: 1,
        correction_actions_attempted: 2,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let current = ReconnectStateRestoreCorrectionMetrics {
        validation_cycles_started: 3,
        correction_actions_attempted: 5,
        correction_retries_scheduled: 2,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };

    let line = reconnect_correction_metrics_delta_json_line(Some(&previous), &current)
        .expect("changed counters should produce a JSON metrics delta line");
    let parsed: Value = serde_json::from_str(&line).expect("metrics delta line should parse");
    assert_eq!(
        parsed,
        json!({
            "type": "reconnect_correction_metrics_delta",
            "deltas": {
                "cycles_started": { "delta": 2, "total": 3 },
                "actions_attempted": { "delta": 3, "total": 5 },
                "retries_scheduled": { "delta": 2, "total": 2 },
            }
        })
    );
    assert_eq!(
        reconnect_correction_metrics_delta_json_line(Some(&current), &current),
        None
    );
}

#[test]
fn reconnect_correction_state_snapshot_message_formats_key_fields() {
    let snapshot = ReconnectStateRestoreCorrectionStateSnapshot {
        validation_pending: true,
        retry_attempts: 2,
        retry_cooldown_ticks: 3,
        mismatch_notified_in_cycle: true,
        mismatch_seen_in_cycle: true,
        effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion,
        position_tolerance_seconds: 1.5,
        effective_retry_max_attempts: 4,
        consecutive_mismatch_cycles: 2,
        consecutive_retry_exhaustions: 1,
        recovery_cooldown_reconnect_cycles_remaining: 0,
        correction_suppressed_for_recovery_cycle: false,
        correction_reenabled_for_recovery_cycle: true,
    };

    let message = reconnect_correction_state_snapshot_message(&snapshot);
    assert!(message.starts_with("reconnect correction state: "));
    assert!(message.contains("pending=true"));
    assert!(message.contains("policy=warn-only-on-exhaustion"));
    assert!(message.contains("tolerance=1.500"));
    assert!(message.contains("retry_attempts=2"));
    assert!(message.contains("effective_retry_max_attempts=4"));
    assert!(message.contains("retry_cooldown_ticks=3"));
    assert!(message.contains("recovery_reenabled_this_cycle=true"));
}

#[test]
fn reconnect_correction_state_snapshot_message_localized_legacy_compatible_localizes_prefix() {
    let snapshot = ReconnectStateRestoreCorrectionStateSnapshot {
        validation_pending: true,
        retry_attempts: 1,
        retry_cooldown_ticks: 0,
        mismatch_notified_in_cycle: false,
        mismatch_seen_in_cycle: false,
        effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect,
        position_tolerance_seconds: 1.0,
        effective_retry_max_attempts: 3,
        consecutive_mismatch_cycles: 0,
        consecutive_retry_exhaustions: 0,
        recovery_cooldown_reconnect_cycles_remaining: 0,
        correction_suppressed_for_recovery_cycle: false,
        correction_reenabled_for_recovery_cycle: false,
    };

    let message = crate::reconnect_correction_state_snapshot_message_localized_legacy_compatible(
        &snapshot,
        Some("es"),
    );
    assert!(message.starts_with("Estado de correccion de reconexion: "));
    assert!(message.contains("policy=auto"));
}

#[test]
fn reconnect_correction_state_snapshot_json_line_formats_key_fields() {
    let snapshot = ReconnectStateRestoreCorrectionStateSnapshot {
        validation_pending: true,
        retry_attempts: 2,
        retry_cooldown_ticks: 3,
        mismatch_notified_in_cycle: true,
        mismatch_seen_in_cycle: true,
        effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion,
        position_tolerance_seconds: 1.5,
        effective_retry_max_attempts: 4,
        consecutive_mismatch_cycles: 2,
        consecutive_retry_exhaustions: 1,
        recovery_cooldown_reconnect_cycles_remaining: 0,
        correction_suppressed_for_recovery_cycle: false,
        correction_reenabled_for_recovery_cycle: true,
    };

    let line = reconnect_correction_state_snapshot_json_line(&snapshot);
    let parsed: Value = serde_json::from_str(&line).expect("state snapshot line should parse");
    assert_eq!(
        parsed,
        json!({
            "type": "reconnect_correction_state",
            "state": {
                "validation_pending": true,
                "effective_policy_mode": "warn-only-on-exhaustion",
                "position_tolerance_seconds": 1.5,
                "retry_attempts": 2,
                "effective_retry_max_attempts": 4,
                "retry_cooldown_ticks": 3,
                "mismatch_notified_in_cycle": true,
                "mismatch_seen_in_cycle": true,
                "consecutive_mismatch_cycles": 2,
                "consecutive_retry_exhaustions": 1,
                "recovery_cooldown_reconnect_cycles_remaining": 0,
                "correction_suppressed_for_recovery_cycle": false,
                "correction_reenabled_for_recovery_cycle": true,
            }
        })
    );
}

#[test]
fn reconnect_correction_metrics_delta_alert_lines_emit_text_and_json_when_thresholds_met() {
    let previous = ReconnectStateRestoreCorrectionMetrics {
        correction_action_failures: 1,
        correction_retry_exhaustions: 1,
        correction_disables_after_repeated_mismatches: 0,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let current = ReconnectStateRestoreCorrectionMetrics {
        correction_action_failures: 3,
        correction_retry_exhaustions: 2,
        correction_disables_after_repeated_mismatches: 1,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let thresholds = crate::ReconnectCorrectionDiagnosticsAlertThresholds {
        action_failures_delta: Some(2),
        retry_exhaustions_delta: Some(2),
        disables_after_repeated_mismatches_delta: Some(1),
        consecutive_mismatch_cycles: None,
        consecutive_retry_exhaustions: None,
    };

    let text_alerts = reconnect_correction_metrics_delta_alert_lines(
        Some(&previous),
        &current,
        &thresholds,
        ReconnectCorrectionDiagnosticsFormat::Text,
    );
    assert_eq!(text_alerts.len(), 2);
    assert!(text_alerts[0].contains("metric=actions_failed"));
    assert!(text_alerts[0].contains("delta=2"));
    assert!(text_alerts[1].contains("metric=disables_after_repeated_mismatches"));

    let json_alerts = reconnect_correction_metrics_delta_alert_lines(
        Some(&previous),
        &current,
        &thresholds,
        ReconnectCorrectionDiagnosticsFormat::Json,
    );
    assert_eq!(json_alerts.len(), 2);
    let first: Value =
        serde_json::from_str(&json_alerts[0]).expect("metrics alert JSON should parse");
    assert_eq!(first["type"], "reconnect_correction_alert");
    assert_eq!(first["alert_kind"], "metric_delta_threshold");
}

#[test]
fn reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible_localize_prefix() {
    let previous = ReconnectStateRestoreCorrectionMetrics {
        correction_action_failures: 1,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let current = ReconnectStateRestoreCorrectionMetrics {
        correction_action_failures: 3,
        ..ReconnectStateRestoreCorrectionMetrics::default()
    };
    let thresholds = crate::ReconnectCorrectionDiagnosticsAlertThresholds {
        action_failures_delta: Some(2),
        ..crate::ReconnectCorrectionDiagnosticsAlertThresholds::default()
    };

    let alerts = crate::reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible(
        Some(&previous),
        &current,
        &thresholds,
        Some("fr"),
    );
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].starts_with("Alerte de correction de reconnexion: "));
    assert!(alerts[0].contains("metric=actions_failed"));
}

#[test]
fn reconnect_correction_state_threshold_alert_lines_only_emit_on_threshold_crossing() {
    let previous = ReconnectStateRestoreCorrectionStateSnapshot {
        validation_pending: true,
        retry_attempts: 0,
        retry_cooldown_ticks: 0,
        mismatch_notified_in_cycle: false,
        mismatch_seen_in_cycle: true,
        effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect,
        position_tolerance_seconds: 1.0,
        effective_retry_max_attempts: 3,
        consecutive_mismatch_cycles: 1,
        consecutive_retry_exhaustions: 1,
        recovery_cooldown_reconnect_cycles_remaining: 0,
        correction_suppressed_for_recovery_cycle: false,
        correction_reenabled_for_recovery_cycle: false,
    };
    let current = ReconnectStateRestoreCorrectionStateSnapshot {
        consecutive_mismatch_cycles: 2,
        consecutive_retry_exhaustions: 2,
        ..previous
    };
    let thresholds = crate::ReconnectCorrectionDiagnosticsAlertThresholds {
        action_failures_delta: None,
        retry_exhaustions_delta: None,
        disables_after_repeated_mismatches_delta: None,
        consecutive_mismatch_cycles: Some(2),
        consecutive_retry_exhaustions: Some(3),
    };

    let text_alerts = reconnect_correction_state_threshold_alert_lines(
        Some(&previous),
        &current,
        &thresholds,
        ReconnectCorrectionDiagnosticsFormat::Text,
    );
    assert_eq!(text_alerts.len(), 1);
    assert!(text_alerts[0].contains("state=consecutive_mismatch_cycles"));

    let json_alerts = reconnect_correction_state_threshold_alert_lines(
        Some(&previous),
        &current,
        &thresholds,
        ReconnectCorrectionDiagnosticsFormat::Json,
    );
    assert_eq!(json_alerts.len(), 1);
    let parsed: Value =
        serde_json::from_str(&json_alerts[0]).expect("state alert JSON should parse");
    assert_eq!(parsed["metric"], "consecutive_mismatch_cycles");
    assert_eq!(parsed["threshold"], 2);
}

#[test]
fn flush_reconnect_correction_diagnostics_to_sink_dedupes_snapshot_and_emits_on_change() {
    let config = test_client_loop_config();
    let mut runtime = create_client_runtime(&config);
    let mut diagnostics_state = ReconnectCorrectionDiagnosticsState::default();
    let mut captured = Vec::new();

    flush_reconnect_correction_diagnostics_to_sink(
        &runtime,
        &mut diagnostics_state,
        &crate::ReconnectCorrectionDiagnosticsAlertThresholds::default(),
        ReconnectCorrectionDiagnosticsFormat::Text,
        &mut |m| {
            captured.push(m.to_owned());
            Ok(())
        },
    )
    .expect("initial reconnect correction diagnostics flush should succeed");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].starts_with("reconnect correction state: "));

    captured.clear();
    flush_reconnect_correction_diagnostics_to_sink(
        &runtime,
        &mut diagnostics_state,
        &crate::ReconnectCorrectionDiagnosticsAlertThresholds::default(),
        ReconnectCorrectionDiagnosticsFormat::Text,
        &mut |m| {
            captured.push(m.to_owned());
            Ok(())
        },
    )
    .expect("deduped reconnect correction diagnostics flush should succeed");
    assert!(
        captured.is_empty(),
        "unchanged reconnect correction metrics/state should not emit duplicate diagnostics"
    );

    runtime
        .session_mut()
        .behavior_config_mut()
        .reconnect_state_restore_auto_correct = false;

    flush_reconnect_correction_diagnostics_to_sink(
        &runtime,
        &mut diagnostics_state,
        &crate::ReconnectCorrectionDiagnosticsAlertThresholds::default(),
        ReconnectCorrectionDiagnosticsFormat::Text,
        &mut |m| {
            captured.push(m.to_owned());
            Ok(())
        },
    )
    .expect("snapshot change should emit reconnect correction diagnostics");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("policy=notify-only"));
}

#[test]
fn flush_reconnect_correction_diagnostics_to_sink_emits_json_lines_when_requested() {
    let config = test_client_loop_config();
    let runtime = create_client_runtime(&config);
    let mut diagnostics_state = ReconnectCorrectionDiagnosticsState::default();
    let mut captured = Vec::new();

    flush_reconnect_correction_diagnostics_to_sink(
        &runtime,
        &mut diagnostics_state,
        &crate::ReconnectCorrectionDiagnosticsAlertThresholds::default(),
        ReconnectCorrectionDiagnosticsFormat::Json,
        &mut |m| {
            captured.push(m.to_owned());
            Ok(())
        },
    )
    .expect("initial JSON reconnect correction diagnostics flush should succeed");

    assert_eq!(captured.len(), 1);
    let parsed: Value = serde_json::from_str(&captured[0])
        .expect("captured reconnect correction JSON should parse");
    assert_eq!(parsed["type"], "reconnect_correction_state");
    assert_eq!(parsed["state"]["effective_policy_mode"], "auto");
}

#[test]
fn reconnect_correction_diagnostics_format_from_env_prefers_json_when_enabled() {
    let key_text = "SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS";
    let key_json = "SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON";
    let old_text = std::env::var(key_text).ok();
    let old_json = std::env::var(key_json).ok();

    // SAFETY: This unit test mutates process env in a short, local scope and restores it
    // before returning; it does not spawn threads or perform concurrent env access.
    unsafe {
        std::env::set_var(key_text, "1");
        std::env::remove_var(key_json);
    }
    assert_eq!(
        reconnect_correction_diagnostics_format_from_env(),
        Some(ReconnectCorrectionDiagnosticsFormat::Text)
    );

    // SAFETY: Same scoped test-only environment mutation reasoning as above.
    unsafe {
        std::env::set_var(key_json, "1");
    }
    assert_eq!(
        reconnect_correction_diagnostics_format_from_env(),
        Some(ReconnectCorrectionDiagnosticsFormat::Json)
    );

    // SAFETY: Same scoped test-only environment mutation reasoning as above.
    unsafe {
        std::env::remove_var(key_text);
        std::env::remove_var(key_json);
    }
    assert_eq!(reconnect_correction_diagnostics_format_from_env(), None);

    if let Some(value) = old_text {
        // SAFETY: Restoring the original test-local env value before exiting the test.
        unsafe {
            std::env::set_var(key_text, value);
        }
    }
    if let Some(value) = old_json {
        // SAFETY: Restoring the original test-local env value before exiting the test.
        unsafe {
            std::env::set_var(key_json, value);
        }
    }
}

#[test]
fn reconnect_correction_diagnostics_alert_thresholds_from_env_parses_values() {
    let key_failures = "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_ACTION_FAILURES_DELTA_THRESHOLD";
    let key_retry = "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_RETRY_EXHAUSTIONS_DELTA_THRESHOLD";
    let key_mismatch =
        "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_CONSECUTIVE_MISMATCH_CYCLES_THRESHOLD";
    let old_failures = std::env::var(key_failures).ok();
    let old_retry = std::env::var(key_retry).ok();
    let old_mismatch = std::env::var(key_mismatch).ok();

    // SAFETY: Scoped unit-test env mutation with restoration before return.
    unsafe {
        std::env::set_var(key_failures, "2");
        std::env::set_var(key_retry, "5");
        std::env::set_var(key_mismatch, "3");
    }

    let thresholds = reconnect_correction_diagnostics_alert_thresholds_from_env();
    assert_eq!(thresholds.action_failures_delta, Some(2));
    assert_eq!(thresholds.retry_exhaustions_delta, Some(5));
    assert_eq!(thresholds.consecutive_mismatch_cycles, Some(3));
    assert_eq!(thresholds.disables_after_repeated_mismatches_delta, None);

    // SAFETY: Scoped unit-test env restoration.
    unsafe {
        std::env::remove_var(key_failures);
        std::env::remove_var(key_retry);
        std::env::remove_var(key_mismatch);
    }
    if let Some(value) = old_failures {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_failures, value);
        }
    }
    if let Some(value) = old_retry {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_retry, value);
        }
    }
    if let Some(value) = old_mismatch {
        // SAFETY: Restoring original env value.
        unsafe {
            std::env::set_var(key_mismatch, value);
        }
    }
}

#[test]
fn apply_legacy_client_arg_diagnostics_overrides_enables_debug_defaults() {
    let config = crate::ClientLoopDiagnosticsConfig {
        log_player_telemetry: false,
        log_player_drift: false,
        reconnect_correction_diagnostics_format: None,
        reconnect_correction_diagnostics_alert_thresholds:
            crate::ReconnectCorrectionDiagnosticsAlertThresholds::default(),
    };
    let overrides = LegacyClientArgOverrides {
        debug_requested: true,
        ..LegacyClientArgOverrides::default()
    };

    let updated = crate::apply_legacy_client_arg_diagnostics_overrides(config, Some(&overrides));

    assert!(updated.log_player_telemetry);
    assert!(updated.log_player_drift);
    assert_eq!(
        updated.reconnect_correction_diagnostics_format,
        Some(ReconnectCorrectionDiagnosticsFormat::Text)
    );
    assert_eq!(
        updated.reconnect_correction_diagnostics_alert_thresholds,
        crate::ReconnectCorrectionDiagnosticsAlertThresholds::default()
    );
}
