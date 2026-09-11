use super::parser::{
    parse_bool_startup_arg, parse_generic_option_assignment_arg, parse_profile_arg,
    parse_speed_arg, parse_start_position_arg, parse_volume_arg,
};
use super::*;

pub(crate) fn analyze_explicit_mpv_ipc_startup_player_args(
    player_args: &[String],
) -> ExplicitMpvIpcStartupPlayerArgAnalysis {
    let mut analysis = ExplicitMpvIpcStartupPlayerArgAnalysis::default();
    let mut index = 0;
    while index < player_args.len() {
        let arg = player_args[index].as_str();

        if parse_bool_startup_arg(arg, &mut analysis) {
            index += 1;
            continue;
        }
        if parse_start_position_arg(player_args, &mut index, &mut analysis)
            || parse_speed_arg(player_args, &mut index, &mut analysis)
            || parse_volume_arg(player_args, &mut index, &mut analysis)
            || parse_profile_arg(player_args, &mut index, &mut analysis)
        {
            continue;
        }
        if parse_generic_option_assignment_arg(arg, &mut analysis) {
            index += 1;
            continue;
        }

        analysis.diagnostics.unsupported_tokens.push(arg.to_owned());
        index += 1;
    }
    analysis
}

#[cfg(test)]
pub(crate) fn parse_explicit_mpv_ipc_startup_player_args(
    player_args: &[String],
) -> ExplicitMpvIpcStartupPlayerArgs {
    analyze_explicit_mpv_ipc_startup_player_args(player_args).parsed
}
