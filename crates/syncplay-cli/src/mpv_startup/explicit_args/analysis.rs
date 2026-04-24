use super::parser::{
    parse_bool_startup_arg_legacy_compatible,
    parse_generic_option_assignment_arg_legacy_compatible, parse_profile_arg_legacy_compatible,
    parse_speed_arg_legacy_compatible, parse_start_position_arg_legacy_compatible,
    parse_volume_arg_legacy_compatible,
};
use super::*;

pub(crate) fn analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(
    player_args: &[String],
) -> LegacyExplicitMpvIpcStartupPlayerArgAnalysis {
    let mut analysis = LegacyExplicitMpvIpcStartupPlayerArgAnalysis::default();
    let mut index = 0;
    while index < player_args.len() {
        let arg = player_args[index].as_str();

        if parse_bool_startup_arg_legacy_compatible(arg, &mut analysis) {
            index += 1;
            continue;
        }
        if parse_start_position_arg_legacy_compatible(player_args, &mut index, &mut analysis)
            || parse_speed_arg_legacy_compatible(player_args, &mut index, &mut analysis)
            || parse_volume_arg_legacy_compatible(player_args, &mut index, &mut analysis)
            || parse_profile_arg_legacy_compatible(player_args, &mut index, &mut analysis)
        {
            continue;
        }
        if parse_generic_option_assignment_arg_legacy_compatible(arg, &mut analysis) {
            index += 1;
            continue;
        }

        analysis.diagnostics.unsupported_tokens.push(arg.to_owned());
        index += 1;
    }
    analysis
}

#[cfg(test)]
pub(crate) fn parse_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(
    player_args: &[String],
) -> LegacyExplicitMpvIpcStartupPlayerArgs {
    analyze_legacy_explicit_mpv_ipc_startup_player_args_legacy_compatible(player_args).parsed
}
