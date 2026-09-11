use super::*;

mod analysis;
mod diagnostics;
mod parser;

pub(crate) use self::analysis::analyze_explicit_mpv_ipc_startup_player_args;
#[cfg(test)]
pub(crate) use self::analysis::parse_explicit_mpv_ipc_startup_player_args;
pub(crate) use self::diagnostics::emit_explicit_mpv_ipc_startup_player_arg_diagnostics;
#[cfg(test)]
pub(crate) use self::diagnostics::explicit_mpv_ipc_startup_player_arg_diagnostic_lines;
