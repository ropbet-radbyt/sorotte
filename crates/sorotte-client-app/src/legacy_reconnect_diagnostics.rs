use serde_json::{Map, Value, json};
use sorotte_client_core::{
    ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionPolicyMode,
    ReconnectStateRestoreCorrectionStateSnapshot,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReconnectCorrectionDiagnosticsState {
    pub last_metrics: Option<ReconnectStateRestoreCorrectionMetrics>,
    pub last_snapshot: Option<ReconnectStateRestoreCorrectionStateSnapshot>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconnectCorrectionDiagnosticsAlertThresholds {
    pub action_failures_delta: Option<u64>,
    pub retry_exhaustions_delta: Option<u64>,
    pub disables_after_repeated_mismatches_delta: Option<u64>,
    pub consecutive_mismatch_cycles: Option<u32>,
    pub consecutive_retry_exhaustions: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectCorrectionDiagnosticsFormat {
    Text,
    Json,
}

fn reconnect_correction_policy_mode_label(
    mode: ReconnectStateRestoreCorrectionPolicyMode,
) -> &'static str {
    match mode {
        ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect => "auto",
        ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly => "notify-only",
        ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion => {
            "warn-only-on-exhaustion"
        }
        ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches => {
            "disable-after-n-mismatches"
        }
    }
}

pub fn reconnect_correction_metrics_delta_message(
    previous: Option<&ReconnectStateRestoreCorrectionMetrics>,
    current: &ReconnectStateRestoreCorrectionMetrics,
) -> Option<String> {
    let baseline = previous.copied().unwrap_or_default();
    let mut fields = Vec::new();

    macro_rules! push_delta {
        ($field:ident, $label:literal) => {
            if current.$field != baseline.$field {
                let delta = current.$field.saturating_sub(baseline.$field);
                fields.push(format!("{}=+{} (total={})", $label, delta, current.$field));
            }
        };
    }

    push_delta!(validation_cycles_started, "cycles_started");
    push_delta!(validation_cycles_completed_without_mismatch, "cycles_clean");
    push_delta!(
        validation_cycles_completed_with_successful_correction,
        "cycles_corrected"
    );
    push_delta!(mismatch_cycles_detected, "mismatch_cycles");
    push_delta!(mismatch_notifications_emitted, "mismatch_notifications");
    push_delta!(correction_actions_attempted, "actions_attempted");
    push_delta!(correction_actions_succeeded, "actions_succeeded");
    push_delta!(correction_action_failures, "actions_failed");
    push_delta!(correction_retries_scheduled, "retries_scheduled");
    push_delta!(correction_retry_exhaustions, "retry_exhaustions");
    push_delta!(
        correction_disables_after_repeated_mismatches,
        "disables_after_repeated_mismatches"
    );
    push_delta!(
        correction_recovery_cooldown_suppressed_cycles,
        "recovery_suppressed_cycles"
    );
    push_delta!(
        correction_recovery_cooldown_reenabled_cycles,
        "recovery_reenabled_cycles"
    );

    if fields.is_empty() {
        None
    } else {
        Some(format!(
            "reconnect correction metrics: {}",
            fields.join(" ")
        ))
    }
}

fn localized_reconnect_correction_metrics_prefix_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Wiederverbindungs-Korrekturmetriken",
        Some("es") => "Metricas de correccion de reconexion",
        Some("eo") => "Rekonektaj korektaj metrikoj",
        Some("fi") => "Uudelleenyhdistamisen korjausmittarit",
        Some("fr") => "Metriques de correction de reconnexion",
        Some("it") => "Metriche di correzione della riconnessione",
        Some("pt_PT" | "pt_BR") => "Metricas de correcao de reconexao",
        Some("tr") => "Yeniden baglanti duzeltme metrikleri",
        Some("ru") => "Metodannye korrektsii povtornogo podkliucheniia",
        Some("zh_CN") => "Chonglian xiuzheng zhibiao",
        Some("ko") => "Dasi yeongyeol bojeong jinpyo",
        _ => "reconnect correction metrics",
    }
}

pub fn reconnect_correction_metrics_delta_message_localized_legacy_compatible(
    previous: Option<&ReconnectStateRestoreCorrectionMetrics>,
    current: &ReconnectStateRestoreCorrectionMetrics,
    language: Option<&str>,
) -> Option<String> {
    if language.is_none() {
        return reconnect_correction_metrics_delta_message(previous, current);
    }

    let baseline = previous.copied().unwrap_or_default();
    let mut fields = Vec::new();

    macro_rules! push_delta {
        ($field:ident, $label:literal) => {
            if current.$field != baseline.$field {
                let delta = current.$field.saturating_sub(baseline.$field);
                fields.push(format!("{}=+{} (total={})", $label, delta, current.$field));
            }
        };
    }

    push_delta!(validation_cycles_started, "cycles_started");
    push_delta!(validation_cycles_completed_without_mismatch, "cycles_clean");
    push_delta!(
        validation_cycles_completed_with_successful_correction,
        "cycles_corrected"
    );
    push_delta!(mismatch_cycles_detected, "mismatch_cycles");
    push_delta!(mismatch_notifications_emitted, "mismatch_notifications");
    push_delta!(correction_actions_attempted, "actions_attempted");
    push_delta!(correction_actions_succeeded, "actions_succeeded");
    push_delta!(correction_action_failures, "actions_failed");
    push_delta!(correction_retries_scheduled, "retries_scheduled");
    push_delta!(correction_retry_exhaustions, "retry_exhaustions");
    push_delta!(
        correction_disables_after_repeated_mismatches,
        "disables_after_repeated_mismatches"
    );
    push_delta!(
        correction_recovery_cooldown_suppressed_cycles,
        "recovery_suppressed_cycles"
    );
    push_delta!(
        correction_recovery_cooldown_reenabled_cycles,
        "recovery_reenabled_cycles"
    );

    if fields.is_empty() {
        None
    } else {
        Some(format!(
            "{}: {}",
            localized_reconnect_correction_metrics_prefix_legacy_compatible(language),
            fields.join(" ")
        ))
    }
}

pub fn reconnect_correction_metrics_delta_json_line(
    previous: Option<&ReconnectStateRestoreCorrectionMetrics>,
    current: &ReconnectStateRestoreCorrectionMetrics,
) -> Option<String> {
    let baseline = previous.copied().unwrap_or_default();
    let mut deltas = Map::new();

    macro_rules! push_delta_json {
        ($field:ident, $label:literal) => {
            if current.$field != baseline.$field {
                let delta = current.$field.saturating_sub(baseline.$field);
                deltas.insert(
                    $label.to_owned(),
                    json!({
                        "delta": delta,
                        "total": current.$field,
                    }),
                );
            }
        };
    }

    push_delta_json!(validation_cycles_started, "cycles_started");
    push_delta_json!(validation_cycles_completed_without_mismatch, "cycles_clean");
    push_delta_json!(
        validation_cycles_completed_with_successful_correction,
        "cycles_corrected"
    );
    push_delta_json!(mismatch_cycles_detected, "mismatch_cycles");
    push_delta_json!(mismatch_notifications_emitted, "mismatch_notifications");
    push_delta_json!(correction_actions_attempted, "actions_attempted");
    push_delta_json!(correction_actions_succeeded, "actions_succeeded");
    push_delta_json!(correction_action_failures, "actions_failed");
    push_delta_json!(correction_retries_scheduled, "retries_scheduled");
    push_delta_json!(correction_retry_exhaustions, "retry_exhaustions");
    push_delta_json!(
        correction_disables_after_repeated_mismatches,
        "disables_after_repeated_mismatches"
    );
    push_delta_json!(
        correction_recovery_cooldown_suppressed_cycles,
        "recovery_suppressed_cycles"
    );
    push_delta_json!(
        correction_recovery_cooldown_reenabled_cycles,
        "recovery_reenabled_cycles"
    );

    if deltas.is_empty() {
        None
    } else {
        Some(
            json!({
                "type": "reconnect_correction_metrics_delta",
                "deltas": Value::Object(deltas),
            })
            .to_string(),
        )
    }
}

pub fn reconnect_correction_state_snapshot_message(
    snapshot: &ReconnectStateRestoreCorrectionStateSnapshot,
) -> String {
    format!(
        "reconnect correction state: pending={} policy={} tolerance={:.3} retry_attempts={} effective_retry_max_attempts={} retry_cooldown_ticks={} mismatch_notified={} mismatch_seen={} consecutive_mismatch_cycles={} consecutive_retry_exhaustions={} recovery_cooldown_reconnect_cycles_remaining={} recovery_suppressed_this_cycle={} recovery_reenabled_this_cycle={}",
        snapshot.validation_pending,
        reconnect_correction_policy_mode_label(snapshot.effective_policy_mode),
        snapshot.position_tolerance_seconds,
        snapshot.retry_attempts,
        snapshot.effective_retry_max_attempts,
        snapshot.retry_cooldown_ticks,
        snapshot.mismatch_notified_in_cycle,
        snapshot.mismatch_seen_in_cycle,
        snapshot.consecutive_mismatch_cycles,
        snapshot.consecutive_retry_exhaustions,
        snapshot.recovery_cooldown_reconnect_cycles_remaining,
        snapshot.correction_suppressed_for_recovery_cycle,
        snapshot.correction_reenabled_for_recovery_cycle,
    )
}

fn localized_reconnect_correction_state_prefix_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Wiederverbindungs-Korrekturstatus",
        Some("es") => "Estado de correccion de reconexion",
        Some("eo") => "Rekonekta korekta stato",
        Some("fi") => "Uudelleenyhdistamisen korjaustila",
        Some("fr") => "Etat de correction de reconnexion",
        Some("it") => "Stato di correzione della riconnessione",
        Some("pt_PT" | "pt_BR") => "Estado da correcao de reconexao",
        Some("tr") => "Yeniden baglanti duzeltme durumu",
        Some("ru") => "Sostoianie korrektsii povtornogo podkliucheniia",
        Some("zh_CN") => "Chonglian xiuzheng zhuangtai",
        Some("ko") => "Dasi yeongyeol bojeong sangtae",
        _ => "reconnect correction state",
    }
}

pub fn reconnect_correction_state_snapshot_message_localized_legacy_compatible(
    snapshot: &ReconnectStateRestoreCorrectionStateSnapshot,
    language: Option<&str>,
) -> String {
    if language.is_none() {
        return reconnect_correction_state_snapshot_message(snapshot);
    }

    format!(
        "{}: pending={} policy={} tolerance={:.3} retry_attempts={} effective_retry_max_attempts={} retry_cooldown_ticks={} mismatch_notified={} mismatch_seen={} consecutive_mismatch_cycles={} consecutive_retry_exhaustions={} recovery_cooldown_reconnect_cycles_remaining={} recovery_suppressed_this_cycle={} recovery_reenabled_this_cycle={}",
        localized_reconnect_correction_state_prefix_legacy_compatible(language),
        snapshot.validation_pending,
        reconnect_correction_policy_mode_label(snapshot.effective_policy_mode),
        snapshot.position_tolerance_seconds,
        snapshot.retry_attempts,
        snapshot.effective_retry_max_attempts,
        snapshot.retry_cooldown_ticks,
        snapshot.mismatch_notified_in_cycle,
        snapshot.mismatch_seen_in_cycle,
        snapshot.consecutive_mismatch_cycles,
        snapshot.consecutive_retry_exhaustions,
        snapshot.recovery_cooldown_reconnect_cycles_remaining,
        snapshot.correction_suppressed_for_recovery_cycle,
        snapshot.correction_reenabled_for_recovery_cycle,
    )
}

pub fn reconnect_correction_state_snapshot_json_line(
    snapshot: &ReconnectStateRestoreCorrectionStateSnapshot,
) -> String {
    json!({
        "type": "reconnect_correction_state",
        "state": {
            "validation_pending": snapshot.validation_pending,
            "effective_policy_mode": reconnect_correction_policy_mode_label(snapshot.effective_policy_mode),
            "position_tolerance_seconds": snapshot.position_tolerance_seconds,
            "retry_attempts": snapshot.retry_attempts,
            "effective_retry_max_attempts": snapshot.effective_retry_max_attempts,
            "retry_cooldown_ticks": snapshot.retry_cooldown_ticks,
            "mismatch_notified_in_cycle": snapshot.mismatch_notified_in_cycle,
            "mismatch_seen_in_cycle": snapshot.mismatch_seen_in_cycle,
            "consecutive_mismatch_cycles": snapshot.consecutive_mismatch_cycles,
            "consecutive_retry_exhaustions": snapshot.consecutive_retry_exhaustions,
            "recovery_cooldown_reconnect_cycles_remaining": snapshot.recovery_cooldown_reconnect_cycles_remaining,
            "correction_suppressed_for_recovery_cycle": snapshot.correction_suppressed_for_recovery_cycle,
            "correction_reenabled_for_recovery_cycle": snapshot.correction_reenabled_for_recovery_cycle,
        }
    })
    .to_string()
}

fn localized_reconnect_correction_alert_prefix_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Wiederverbindungs-Korrekturwarnung",
        Some("es") => "Alerta de correccion de reconexion",
        Some("eo") => "Rekonekta korekta averto",
        Some("fi") => "Uudelleenyhdistamisen korjaushalytys",
        Some("fr") => "Alerte de correction de reconnexion",
        Some("it") => "Avviso di correzione della riconnessione",
        Some("pt_PT" | "pt_BR") => "Alerta de correcao de reconexao",
        Some("tr") => "Yeniden baglanti duzeltme uyarisi",
        Some("ru") => "Preduprezhdenie korrektsii povtornogo podkliucheniia",
        Some("zh_CN") => "Chonglian xiuzheng jinggao",
        Some("ko") => "Dasi yeongyeol bojeong gyeonggo",
        _ => "reconnect correction alert",
    }
}

fn reconnect_correction_metric_delta_alert_text(
    metric: &str,
    delta: u64,
    total: u64,
    threshold: u64,
) -> String {
    format!(
        "reconnect correction alert: metric={metric} delta={delta} total={total} threshold={threshold}"
    )
}

fn reconnect_correction_metric_delta_alert_text_localized_legacy_compatible(
    metric: &str,
    delta: u64,
    total: u64,
    threshold: u64,
    language: Option<&str>,
) -> String {
    format!(
        "{}: metric={metric} delta={delta} total={total} threshold={threshold}",
        localized_reconnect_correction_alert_prefix_legacy_compatible(language)
    )
}

fn reconnect_correction_metric_delta_alert_json_line(
    metric: &str,
    delta: u64,
    total: u64,
    threshold: u64,
) -> String {
    json!({
        "type": "reconnect_correction_alert",
        "alert_kind": "metric_delta_threshold",
        "metric": metric,
        "delta": delta,
        "total": total,
        "threshold": threshold,
    })
    .to_string()
}

fn reconnect_correction_state_threshold_alert_text(
    metric: &str,
    value: u32,
    threshold: u32,
) -> String {
    format!("reconnect correction alert: state={metric} value={value} threshold={threshold}")
}

fn reconnect_correction_state_threshold_alert_text_localized_legacy_compatible(
    metric: &str,
    value: u32,
    threshold: u32,
    language: Option<&str>,
) -> String {
    format!(
        "{}: state={metric} value={value} threshold={threshold}",
        localized_reconnect_correction_alert_prefix_legacy_compatible(language)
    )
}

fn reconnect_correction_state_threshold_alert_json_line(
    metric: &str,
    value: u32,
    threshold: u32,
) -> String {
    json!({
        "type": "reconnect_correction_alert",
        "alert_kind": "state_threshold_crossed",
        "metric": metric,
        "value": value,
        "threshold": threshold,
    })
    .to_string()
}

pub fn reconnect_correction_metrics_delta_alert_lines(
    previous: Option<&ReconnectStateRestoreCorrectionMetrics>,
    current: &ReconnectStateRestoreCorrectionMetrics,
    thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    format: ReconnectCorrectionDiagnosticsFormat,
) -> Vec<String> {
    let baseline = previous.copied().unwrap_or_default();
    let mut alerts = Vec::new();

    macro_rules! push_metric_alert {
        ($threshold_field:ident, $metric_field:ident, $metric_label:literal) => {
            if let Some(threshold) = thresholds.$threshold_field {
                let current_total = current.$metric_field;
                let baseline_total = baseline.$metric_field;
                let delta = current_total.saturating_sub(baseline_total);
                if delta >= threshold && delta > 0 {
                    let message = match format {
                        ReconnectCorrectionDiagnosticsFormat::Text => {
                            reconnect_correction_metric_delta_alert_text(
                                $metric_label,
                                delta,
                                current_total,
                                threshold,
                            )
                        }
                        ReconnectCorrectionDiagnosticsFormat::Json => {
                            reconnect_correction_metric_delta_alert_json_line(
                                $metric_label,
                                delta,
                                current_total,
                                threshold,
                            )
                        }
                    };
                    alerts.push(message);
                }
            }
        };
    }

    push_metric_alert!(
        action_failures_delta,
        correction_action_failures,
        "actions_failed"
    );
    push_metric_alert!(
        retry_exhaustions_delta,
        correction_retry_exhaustions,
        "retry_exhaustions"
    );
    push_metric_alert!(
        disables_after_repeated_mismatches_delta,
        correction_disables_after_repeated_mismatches,
        "disables_after_repeated_mismatches"
    );

    alerts
}

pub fn reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible(
    previous: Option<&ReconnectStateRestoreCorrectionMetrics>,
    current: &ReconnectStateRestoreCorrectionMetrics,
    thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    language: Option<&str>,
) -> Vec<String> {
    let baseline = previous.copied().unwrap_or_default();
    let mut alerts = Vec::new();

    macro_rules! push_metric_alert {
        ($threshold_field:ident, $metric_field:ident, $metric_label:literal) => {
            if let Some(threshold) = thresholds.$threshold_field {
                let delta = current.$metric_field.saturating_sub(baseline.$metric_field);
                if delta >= threshold && delta > 0 {
                    alerts.push(
                        reconnect_correction_metric_delta_alert_text_localized_legacy_compatible(
                            $metric_label,
                            delta,
                            current.$metric_field,
                            threshold,
                            language,
                        ),
                    );
                }
            }
        };
    }

    push_metric_alert!(
        action_failures_delta,
        correction_action_failures,
        "actions_failed"
    );
    push_metric_alert!(
        retry_exhaustions_delta,
        correction_retry_exhaustions,
        "retry_exhaustions"
    );
    push_metric_alert!(
        disables_after_repeated_mismatches_delta,
        correction_disables_after_repeated_mismatches,
        "disables_after_repeated_mismatches"
    );

    alerts
}

pub fn reconnect_correction_state_threshold_alert_lines(
    previous: Option<&ReconnectStateRestoreCorrectionStateSnapshot>,
    current: &ReconnectStateRestoreCorrectionStateSnapshot,
    thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    format: ReconnectCorrectionDiagnosticsFormat,
) -> Vec<String> {
    let mut alerts = Vec::new();

    macro_rules! push_crossing_alert {
        ($threshold_field:ident, $snapshot_field:ident, $metric_label:literal) => {
            if let Some(threshold) = thresholds.$threshold_field {
                let previous_value = previous
                    .map(|snapshot| snapshot.$snapshot_field)
                    .unwrap_or(0);
                let current_value = current.$snapshot_field;
                if previous_value < threshold && current_value >= threshold {
                    let message = match format {
                        ReconnectCorrectionDiagnosticsFormat::Text => {
                            reconnect_correction_state_threshold_alert_text(
                                $metric_label,
                                current_value,
                                threshold,
                            )
                        }
                        ReconnectCorrectionDiagnosticsFormat::Json => {
                            reconnect_correction_state_threshold_alert_json_line(
                                $metric_label,
                                current_value,
                                threshold,
                            )
                        }
                    };
                    alerts.push(message);
                }
            }
        };
    }

    push_crossing_alert!(
        consecutive_mismatch_cycles,
        consecutive_mismatch_cycles,
        "consecutive_mismatch_cycles"
    );
    push_crossing_alert!(
        consecutive_retry_exhaustions,
        consecutive_retry_exhaustions,
        "consecutive_retry_exhaustions"
    );

    alerts
}

fn reconnect_correction_state_threshold_alert_lines_localized_legacy_compatible(
    previous: Option<&ReconnectStateRestoreCorrectionStateSnapshot>,
    current: &ReconnectStateRestoreCorrectionStateSnapshot,
    thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    language: Option<&str>,
) -> Vec<String> {
    let mut alerts = Vec::new();

    macro_rules! push_crossing_alert {
        ($threshold_field:ident, $snapshot_field:ident, $metric_label:literal) => {
            if let Some(threshold) = thresholds.$threshold_field {
                let previous_value = previous
                    .map(|snapshot| snapshot.$snapshot_field)
                    .unwrap_or(0);
                let current_value = current.$snapshot_field;
                if previous_value < threshold && current_value >= threshold {
                    alerts.push(
                        reconnect_correction_state_threshold_alert_text_localized_legacy_compatible(
                            $metric_label,
                            current_value,
                            threshold,
                            language,
                        ),
                    );
                }
            }
        };
    }

    push_crossing_alert!(
        consecutive_mismatch_cycles,
        consecutive_mismatch_cycles,
        "consecutive_mismatch_cycles"
    );
    push_crossing_alert!(
        consecutive_retry_exhaustions,
        consecutive_retry_exhaustions,
        "consecutive_retry_exhaustions"
    );

    alerts
}

pub fn next_reconnect_correction_diagnostic_lines_legacy_compatible(
    state: &mut ReconnectCorrectionDiagnosticsState,
    metrics: ReconnectStateRestoreCorrectionMetrics,
    snapshot: ReconnectStateRestoreCorrectionStateSnapshot,
    alert_thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    format: ReconnectCorrectionDiagnosticsFormat,
    language: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();

    let metrics_message = match format {
        ReconnectCorrectionDiagnosticsFormat::Text => {
            reconnect_correction_metrics_delta_message_localized_legacy_compatible(
                state.last_metrics.as_ref(),
                &metrics,
                language,
            )
        }
        ReconnectCorrectionDiagnosticsFormat::Json => {
            reconnect_correction_metrics_delta_json_line(state.last_metrics.as_ref(), &metrics)
        }
    };
    if let Some(message) = metrics_message {
        lines.push(message);
    }

    let metric_alerts = match format {
        ReconnectCorrectionDiagnosticsFormat::Text => {
            reconnect_correction_metrics_delta_alert_lines_localized_legacy_compatible(
                state.last_metrics.as_ref(),
                &metrics,
                alert_thresholds,
                language,
            )
        }
        ReconnectCorrectionDiagnosticsFormat::Json => {
            reconnect_correction_metrics_delta_alert_lines(
                state.last_metrics.as_ref(),
                &metrics,
                alert_thresholds,
                format,
            )
        }
    };
    lines.extend(metric_alerts);

    if state.last_snapshot.as_ref() != Some(&snapshot) {
        let message = match format {
            ReconnectCorrectionDiagnosticsFormat::Text => {
                reconnect_correction_state_snapshot_message_localized_legacy_compatible(
                    &snapshot, language,
                )
            }
            ReconnectCorrectionDiagnosticsFormat::Json => {
                reconnect_correction_state_snapshot_json_line(&snapshot)
            }
        };
        lines.push(message);
    }

    let state_alerts = match format {
        ReconnectCorrectionDiagnosticsFormat::Text => {
            reconnect_correction_state_threshold_alert_lines_localized_legacy_compatible(
                state.last_snapshot.as_ref(),
                &snapshot,
                alert_thresholds,
                language,
            )
        }
        ReconnectCorrectionDiagnosticsFormat::Json => {
            reconnect_correction_state_threshold_alert_lines(
                state.last_snapshot.as_ref(),
                &snapshot,
                alert_thresholds,
                format,
            )
        }
    };
    lines.extend(state_alerts);

    state.last_metrics = Some(metrics);
    state.last_snapshot = Some(snapshot);
    lines
}

#[cfg(test)]
mod tests {
    use sorotte_client_core::{
        ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionPolicyMode,
        ReconnectStateRestoreCorrectionStateSnapshot,
    };

    use super::{
        ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
        ReconnectCorrectionDiagnosticsState,
        next_reconnect_correction_diagnostic_lines_legacy_compatible,
    };

    fn snapshot(
        effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode,
        consecutive_mismatch_cycles: u32,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        ReconnectStateRestoreCorrectionStateSnapshot {
            validation_pending: false,
            retry_attempts: 0,
            retry_cooldown_ticks: 0,
            mismatch_notified_in_cycle: false,
            mismatch_seen_in_cycle: false,
            effective_policy_mode,
            position_tolerance_seconds: 0.0,
            effective_retry_max_attempts: 0,
            consecutive_mismatch_cycles,
            consecutive_retry_exhaustions: 0,
            recovery_cooldown_reconnect_cycles_remaining: 0,
            correction_suppressed_for_recovery_cycle: false,
            correction_reenabled_for_recovery_cycle: false,
        }
    }

    #[test]
    fn next_reconnect_correction_diagnostic_lines_emits_metrics_and_snapshot_once() {
        let mut state = ReconnectCorrectionDiagnosticsState::default();
        let lines = next_reconnect_correction_diagnostic_lines_legacy_compatible(
            &mut state,
            ReconnectStateRestoreCorrectionMetrics {
                validation_cycles_started: 1,
                ..ReconnectStateRestoreCorrectionMetrics::default()
            },
            snapshot(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly, 0),
            &ReconnectCorrectionDiagnosticsAlertThresholds::default(),
            ReconnectCorrectionDiagnosticsFormat::Text,
            None,
        );

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("reconnect correction metrics:"));
        assert!(lines[1].starts_with("reconnect correction state:"));
    }

    #[test]
    fn next_reconnect_correction_diagnostic_lines_honors_threshold_alerts_and_dedupes_snapshot() {
        let mut state = ReconnectCorrectionDiagnosticsState::default();
        let snapshot = snapshot(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly, 2);

        let first = next_reconnect_correction_diagnostic_lines_legacy_compatible(
            &mut state,
            ReconnectStateRestoreCorrectionMetrics {
                correction_action_failures: 2,
                ..ReconnectStateRestoreCorrectionMetrics::default()
            },
            snapshot,
            &ReconnectCorrectionDiagnosticsAlertThresholds {
                action_failures_delta: Some(2),
                consecutive_mismatch_cycles: Some(2),
                ..ReconnectCorrectionDiagnosticsAlertThresholds::default()
            },
            ReconnectCorrectionDiagnosticsFormat::Json,
            None,
        );
        assert_eq!(first.len(), 4);

        let second = next_reconnect_correction_diagnostic_lines_legacy_compatible(
            &mut state,
            ReconnectStateRestoreCorrectionMetrics {
                correction_action_failures: 2,
                ..ReconnectStateRestoreCorrectionMetrics::default()
            },
            snapshot,
            &ReconnectCorrectionDiagnosticsAlertThresholds {
                action_failures_delta: Some(2),
                consecutive_mismatch_cycles: Some(2),
                ..ReconnectCorrectionDiagnosticsAlertThresholds::default()
            },
            ReconnectCorrectionDiagnosticsFormat::Json,
            None,
        );
        assert!(second.is_empty());
    }
}
