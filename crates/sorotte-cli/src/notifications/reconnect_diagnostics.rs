use super::*;

pub(crate) fn emit_reconnect_correction_diagnostic(message: &str) -> anyhow::Result<()> {
    println!("{message}");
    Ok(())
}

pub(crate) fn flush_reconnect_correction_diagnostics_to_sink<F>(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    state: &mut ReconnectCorrectionDiagnosticsState,
    alert_thresholds: &ReconnectCorrectionDiagnosticsAlertThresholds,
    format: ReconnectCorrectionDiagnosticsFormat,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> anyhow::Result<()>,
{
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let lines = shared_next_reconnect_correction_diagnostic_lines_legacy_compatible(
        state,
        *runtime.reconnect_state_restore_correction_metrics(),
        runtime.reconnect_state_restore_correction_state_snapshot(),
        alert_thresholds,
        format,
        language.as_deref(),
    );
    for line in lines {
        notify(&line)?;
    }
    Ok(())
}
