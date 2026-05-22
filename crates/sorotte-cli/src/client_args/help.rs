use super::localization::{
    localized_compatibility_field_label_legacy_compatible,
    localized_compatibility_input_label_legacy_compatible,
    localized_compatibility_note_label_legacy_compatible,
    localized_compatibility_status_label_legacy_compatible,
    localized_legacy_ini_compatibility_heading_legacy_compatible,
    localized_legacy_startup_compatibility_heading_legacy_compatible,
};
use super::*;

pub(crate) fn print_legacy_client_help(language: Option<&str>) {
    if let Some(line) = legacy_runtime_language_selection_line_legacy_compatible(language) {
        println!("{line}");
        println!();
    }
    let help_lines = [
        "Usage: sorotte-cli [OPTIONS]",
        "  --no-gui",
        "  -a, --host <hostname[:port]>",
        "  -n, --name <username>",
        "  -r, --room [room]",
        "  -p, --password [password]",
        "  --player-path <path>",
        "  -d, --debug",
        "  -g, --force-gui-prompt",
        "  --language <language>",
        "    Supported values: de/en/es/eo/fi/fr/it/pt_PT/pt_BR/tr/ru/zh_CN/ko",
        "  [file] [options...]",
        "  --clear-gui-data",
        "  --config-path <path>",
        "    Use this exact sorotte.ini path for stored settings.",
        "  --config-root <path>",
        "    Store sorotte.ini and colocated GUI data under this folder.",
        "  --load-playlist-from-file <path>",
        "  --no-store",
        "  -v, --version",
        "  -h, --help",
        "",
        "Environment (optional mpv integration / diagnostics):",
        "  SOROTTE_CLIENT_CONFIG_PATH=<path>",
        "    Use this exact sorotte.ini path when no CLI config path/root is set.",
        "  SOROTTE_CLIENT_CONFIG_ROOT=<path>",
        "    Store sorotte.ini and colocated GUI data under this folder when no CLI override is set.",
        "  SOROTTE_CLIENT_MPV_IPC_PATH=<path>",
        "    Attach to an existing mpv JSON IPC socket/pipe (fallback: SOROTTE_MPV_IPC_PATH).",
        "  SOROTTE_CLIENT_MPV_MANAGED_LAUNCH=1",
        "    Start a managed mpv process and auto-attach its JSON IPC (ignored when explicit IPC path is set).",
        "  SOROTTE_CLIENT_MPV_MANAGED_BIN=<path>",
        "    mpv binary for managed launch (defaults to a repo-local ./mpv/mpv(.exe) when found).",
        "  SOROTTE_CLIENT_MPV_MANAGED_MEDIA=<path>",
        "    Optional media file to preload when launching managed mpv.",
        "  SOROTTE_CLIENT_MPV_MANAGED_IPC_PATH=<path>",
        "    Optional JSON IPC socket/pipe path for managed mpv (auto-generated when omitted).",
        "  SOROTTE_CLIENT_MPV_MANAGED_CONNECT_TIMEOUT_MS=<ms>",
        "    Max wait for managed mpv JSON IPC to become connectable (default 5000).",
        "  SOROTTE_CLIENT_MPV_MANAGED_CONNECT_POLL_INTERVAL_MS=<ms>",
        "    Poll interval while waiting for managed mpv JSON IPC (default 50).",
        "  SOROTTE_CLIENT_PLEX_SYNC=1",
        "    Enable one-way Plex watch sync when token and selected server settings are available.",
        "  SOROTTE_CLIENT_PLEX_TOKEN=<token>",
        "    Plex user token for server discovery fallback and selected-server access.",
        "  SOROTTE_CLIENT_PLEX_SERVER_URL=<url>",
        "    Selected Plex Media Server base URL, e.g. http://127.0.0.1:32400.",
        "  SOROTTE_CLIENT_PLEX_SERVER_TOKEN=<token>",
        "    Access token for the selected Plex Media Server (falls back to SOROTTE_CLIENT_PLEX_TOKEN).",
        "  SOROTTE_CLIENT_LOG_PLAYER_TELEMETRY=1",
        "    Print raw player telemetry updates (pause/position/speed) when available.",
        "  SOROTTE_CLIENT_LOG_PLAYER_DRIFT_DIAGNOSTICS=1",
        "    Print read-only player-vs-room drift diagnostics (no behavior change).",
        "  SOROTTE_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS=1",
        "    Print reconnect correction metrics deltas and policy-state snapshots for diagnostics.",
        "  SOROTTE_CLIENT_LOG_RECONNECT_CORRECTION_DIAGNOSTICS_JSON=1",
        "    Emit reconnect correction diagnostics as JSON lines (implies diagnostics enabled).",
        "  SOROTTE_CLIENT_RECONNECT_CORRECTION_ALERT_ACTION_FAILURES_DELTA_THRESHOLD=<count>",
        "    Emit reconnect diagnostics alerts when action-failure deltas meet/exceed this threshold.",
        "  SOROTTE_CLIENT_RECONNECT_CORRECTION_ALERT_RETRY_EXHAUSTIONS_DELTA_THRESHOLD=<count>",
        "    Emit reconnect diagnostics alerts when retry-exhaustion deltas meet/exceed this threshold.",
        "  SOROTTE_CLIENT_RECONNECT_CORRECTION_ALERT_DISABLES_DELTA_THRESHOLD=<count>",
        "    Emit reconnect diagnostics alerts when correction-disable deltas meet/exceed this threshold.",
        "  SOROTTE_CLIENT_RECONNECT_CORRECTION_ALERT_CONSECUTIVE_MISMATCH_CYCLES_THRESHOLD=<count>",
        "    Emit reconnect diagnostics alerts when consecutive mismatch cycles cross this threshold.",
        "  SOROTTE_CLIENT_RECONNECT_CORRECTION_ALERT_CONSECUTIVE_RETRY_EXHAUSTIONS_THRESHOLD=<count>",
        "    Emit reconnect diagnostics alerts when consecutive retry exhaustions cross this threshold.",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_AUTOCORRECT=0|1",
        "    Control reconnect mismatch policy (default auto-correct on; set 0 for warning-only).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_POLICY=auto|notify-only|warn-only-on-exhaustion",
        "    Explicit reconnect correction policy mode (overrides legacy AUTOCORRECT flag when set).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_POSITION_TOLERANCE_SECONDS=<seconds>",
        "    Position mismatch tolerance for reconnect validation/correction (default 1.0).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS=<count>",
        "    Max reconnect correction failures before giving up (default 3; 0 disables retries).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS=<ticks>",
        "    Validation invocations to wait before retrying reconnect correction (default 1).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF=0|1",
        "    Use exponential cooldown growth between reconnect correction retries (default 0).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS=<ticks>",
        "    Max cooldown cap used when exponential reconnect correction retry backoff is enabled (default 8).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF=0|1",
        "    Scale reconnect correction retry cooldowns across reconnect cycles after retry exhaustion (default 0).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET=0|1",
        "    Reduce reconnect correction retry budget across reconnect cycles after retry exhaustion (default 0).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS=<count>",
        "    Minimum retry budget preserved when adaptive cycle retry-budget reduction is enabled (default 0).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES=<count>",
        "    In `disable-after-n-mismatches` mode, disable correction after this many consecutive restore mismatch cycles (default 0 = disabled).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS=<count>",
        "    Reduce the repeated mismatch counter after a successful reconnect correction (default 0 = no decay).",
        "  SOROTTE_CLIENT_RECONNECT_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES=<count>",
        "    After correction give-up/disable, suppress correction for this many reconnect restore cycles before re-enabling (default 0).",
    ];
    for line in help_lines {
        println!("{line}");
    }
    println!();
    println!(
        "{}",
        localized_legacy_startup_compatibility_heading_legacy_compatible(language)
    );
    println!(
        "  {:<26} {:<10} {}",
        localized_compatibility_input_label_legacy_compatible(language),
        localized_compatibility_status_label_legacy_compatible(language),
        localized_compatibility_note_label_legacy_compatible(language),
    );
    for entry in legacy_configuration_getter_startup_compat_entries() {
        println!(
            "  {:<26} {:<10} {}",
            entry.input,
            entry.status.as_str(),
            entry.note
        );
    }
    println!();
    println!(
        "{}",
        localized_legacy_ini_compatibility_heading_legacy_compatible(language)
    );
    println!(
        "  {:<66} {:<10} {}",
        localized_compatibility_field_label_legacy_compatible(language),
        localized_compatibility_status_label_legacy_compatible(language),
        localized_compatibility_note_label_legacy_compatible(language),
    );
    for entry in legacy_configuration_getter_ini_compat_entries() {
        println!(
            "  {:<66} {:<10} {}",
            entry.key,
            entry.status.as_str(),
            entry.note
        );
    }
}
