use super::*;

pub(crate) fn legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible(
    diagnostics: &LegacyExplicitMpvIpcStartupPlayerArgDiagnostics,
    applied_supported_commands: usize,
) -> Vec<String> {
    let ignored_count = diagnostics.malformed_tokens.len() + diagnostics.unsupported_tokens.len();
    if applied_supported_commands == 0 && ignored_count == 0 {
        return Vec::new();
    }
    let recognized_supported_count = diagnostics.supported_tokens.len();

    let mut lines = Vec::new();
    lines.push(format!(
        "info: explicit-mpv-IPC startup _args summary: applied={applied_supported_commands} ignored={ignored_count} (recognized-supported-tokens={recognized_supported_count}, malformed={}, unsupported={})",
        diagnostics.malformed_tokens.len(),
        diagnostics.unsupported_tokens.len()
    ));
    if !diagnostics.malformed_tokens.is_empty() {
        lines.push(format!(
            "warning: explicit-mpv-IPC malformed _args were ignored: {}",
            RedactedCommandArgs::from_args(&diagnostics.malformed_tokens)
        ));
    }
    if !diagnostics.unsupported_tokens.is_empty() {
        lines.push(format!(
            "warning: explicit-mpv-IPC launch-only _args were ignored in attach mode: {}",
            RedactedCommandArgs::from_args(&diagnostics.unsupported_tokens)
        ));
    }
    lines
}

pub(crate) fn emit_legacy_explicit_mpv_ipc_startup_player_arg_diagnostics_legacy_compatible(
    diagnostics: &LegacyExplicitMpvIpcStartupPlayerArgDiagnostics,
    applied_supported_commands: usize,
) {
    for line in legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible(
        diagnostics,
        applied_supported_commands,
    ) {
        eprintln!("{line}");
    }
}
