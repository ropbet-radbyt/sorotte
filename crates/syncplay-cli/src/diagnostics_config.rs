use syncplay_client_app::app_boundary::diagnostics::{
    ReconnectCorrectionDiagnosticsAlertThresholds, ReconnectCorrectionDiagnosticsFormat,
};

use crate::client_args::LegacyClientArgOverrides;
use crate::env_support::{env_flag_enabled, env_u32};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClientLoopDiagnosticsConfig {
    pub(super) log_player_telemetry: bool,
    pub(super) log_player_drift: bool,
    pub(super) reconnect_correction_diagnostics_format:
        Option<ReconnectCorrectionDiagnosticsFormat>,
    pub(super) reconnect_correction_diagnostics_alert_thresholds:
        ReconnectCorrectionDiagnosticsAlertThresholds,
}

pub(super) fn reconnect_correction_diagnostics_format_from_env()
-> Option<ReconnectCorrectionDiagnosticsFormat> {
    if env_flag_enabled("SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON") {
        return Some(ReconnectCorrectionDiagnosticsFormat::Json);
    }
    if env_flag_enabled("SYNCPLAY_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS") {
        return Some(ReconnectCorrectionDiagnosticsFormat::Text);
    }
    None
}

pub(super) fn reconnect_correction_diagnostics_alert_thresholds_from_env()
-> ReconnectCorrectionDiagnosticsAlertThresholds {
    ReconnectCorrectionDiagnosticsAlertThresholds {
        action_failures_delta: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_ACTION_FAILURES_DELTA_THRESHOLD",
        )
        .map(u64::from),
        retry_exhaustions_delta: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_RETRY_EXHAUSTIONS_DELTA_THRESHOLD",
        )
        .map(u64::from),
        disables_after_repeated_mismatches_delta: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_DISABLES_DELTA_THRESHOLD",
        )
        .map(u64::from),
        consecutive_mismatch_cycles: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_CONSECUTIVE_MISMATCH_CYCLES_THRESHOLD",
        ),
        consecutive_retry_exhaustions: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_CORRECTION_ALERT_CONSECUTIVE_RETRY_EXHAUSTIONS_THRESHOLD",
        ),
    }
}

fn client_loop_diagnostics_config_from_env() -> ClientLoopDiagnosticsConfig {
    ClientLoopDiagnosticsConfig {
        log_player_telemetry: env_flag_enabled("SYNCPLAY_CLIENT_LOG_PLAYER_TELEMETRY"),
        log_player_drift: env_flag_enabled("SYNCPLAY_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS"),
        reconnect_correction_diagnostics_format: reconnect_correction_diagnostics_format_from_env(),
        reconnect_correction_diagnostics_alert_thresholds:
            reconnect_correction_diagnostics_alert_thresholds_from_env(),
    }
}

pub(super) fn apply_legacy_client_arg_diagnostics_overrides(
    mut config: ClientLoopDiagnosticsConfig,
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> ClientLoopDiagnosticsConfig {
    if legacy_overrides.is_some_and(|overrides| overrides.debug_requested) {
        config.log_player_telemetry = true;
        config.log_player_drift = true;
        if config.reconnect_correction_diagnostics_format.is_none() {
            config.reconnect_correction_diagnostics_format =
                Some(ReconnectCorrectionDiagnosticsFormat::Text);
        }
    }
    config
}

pub(super) fn client_loop_diagnostics_config(
    legacy_overrides: Option<&LegacyClientArgOverrides>,
) -> ClientLoopDiagnosticsConfig {
    apply_legacy_client_arg_diagnostics_overrides(
        client_loop_diagnostics_config_from_env(),
        legacy_overrides,
    )
}
