use super::*;

mod analysis;
mod diagnostics;
mod parser;

pub(crate) use self::analysis::analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible;
#[cfg(test)]
pub(crate) use self::analysis::parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible;
pub(crate) use self::diagnostics::emit_legacy_explicit_mpv_ipc_startup_player_arg_diagnostics_legacy_compatible;
#[cfg(test)]
pub(crate) use self::diagnostics::legacy_explicit_mpv_ipc_startup_player_arg_diagnostic_lines_legacy_compatible;
