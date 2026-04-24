use super::*;

#[derive(Debug, Clone, Copy)]
enum BoolStartupArgTarget {
    Paused,
    Muted,
    Deinterlace,
    Keepaspect,
    KeepaspectWindow,
    Fullscreen,
    Ontop,
    Border,
    ForceWindow,
    KeepOpen,
    KeepOpenPause,
    CursorAutohideFsOnly,
    StopScreensaver,
    SubVisibility,
    OsdBar,
    WindowMaximized,
    WindowMinimized,
}

#[derive(Debug, Clone, Copy)]
struct BoolStartupArgSpec {
    positive_flags: &'static [&'static str],
    negative_flags: &'static [&'static str],
    assignment_prefixes: &'static [&'static str],
    target: BoolStartupArgTarget,
}

const BOOL_STARTUP_ARG_SPECS: &[BoolStartupArgSpec] = &[
    BoolStartupArgSpec {
        positive_flags: &["--pause"],
        negative_flags: &["--no-pause"],
        assignment_prefixes: &["--pause="],
        target: BoolStartupArgTarget::Paused,
    },
    BoolStartupArgSpec {
        positive_flags: &["--mute"],
        negative_flags: &["--no-mute"],
        assignment_prefixes: &["--mute="],
        target: BoolStartupArgTarget::Muted,
    },
    BoolStartupArgSpec {
        positive_flags: &["--deinterlace"],
        negative_flags: &["--no-deinterlace"],
        assignment_prefixes: &["--deinterlace="],
        target: BoolStartupArgTarget::Deinterlace,
    },
    BoolStartupArgSpec {
        positive_flags: &["--keepaspect"],
        negative_flags: &["--no-keepaspect"],
        assignment_prefixes: &["--keepaspect="],
        target: BoolStartupArgTarget::Keepaspect,
    },
    BoolStartupArgSpec {
        positive_flags: &["--keepaspect-window"],
        negative_flags: &["--no-keepaspect-window"],
        assignment_prefixes: &["--keepaspect-window="],
        target: BoolStartupArgTarget::KeepaspectWindow,
    },
    BoolStartupArgSpec {
        positive_flags: &["--fs", "--fullscreen"],
        negative_flags: &["--no-fs", "--no-fullscreen"],
        assignment_prefixes: &["--fs=", "--fullscreen="],
        target: BoolStartupArgTarget::Fullscreen,
    },
    BoolStartupArgSpec {
        positive_flags: &["--ontop"],
        negative_flags: &["--no-ontop"],
        assignment_prefixes: &["--ontop="],
        target: BoolStartupArgTarget::Ontop,
    },
    BoolStartupArgSpec {
        positive_flags: &["--border"],
        negative_flags: &["--no-border"],
        assignment_prefixes: &["--border="],
        target: BoolStartupArgTarget::Border,
    },
    BoolStartupArgSpec {
        positive_flags: &["--force-window"],
        negative_flags: &["--no-force-window"],
        assignment_prefixes: &["--force-window="],
        target: BoolStartupArgTarget::ForceWindow,
    },
    BoolStartupArgSpec {
        positive_flags: &["--keep-open"],
        negative_flags: &["--no-keep-open"],
        assignment_prefixes: &["--keep-open="],
        target: BoolStartupArgTarget::KeepOpen,
    },
    BoolStartupArgSpec {
        positive_flags: &["--keep-open-pause"],
        negative_flags: &["--no-keep-open-pause"],
        assignment_prefixes: &["--keep-open-pause="],
        target: BoolStartupArgTarget::KeepOpenPause,
    },
    BoolStartupArgSpec {
        positive_flags: &["--cursor-autohide-fs-only"],
        negative_flags: &["--no-cursor-autohide-fs-only"],
        assignment_prefixes: &["--cursor-autohide-fs-only="],
        target: BoolStartupArgTarget::CursorAutohideFsOnly,
    },
    BoolStartupArgSpec {
        positive_flags: &["--stop-screensaver"],
        negative_flags: &["--no-stop-screensaver"],
        assignment_prefixes: &["--stop-screensaver="],
        target: BoolStartupArgTarget::StopScreensaver,
    },
    BoolStartupArgSpec {
        positive_flags: &["--sub-visibility"],
        negative_flags: &["--no-sub-visibility"],
        assignment_prefixes: &["--sub-visibility="],
        target: BoolStartupArgTarget::SubVisibility,
    },
    BoolStartupArgSpec {
        positive_flags: &["--osd-bar"],
        negative_flags: &["--no-osd-bar"],
        assignment_prefixes: &["--osd-bar="],
        target: BoolStartupArgTarget::OsdBar,
    },
    BoolStartupArgSpec {
        positive_flags: &["--window-maximized"],
        negative_flags: &["--no-window-maximized"],
        assignment_prefixes: &["--window-maximized="],
        target: BoolStartupArgTarget::WindowMaximized,
    },
    BoolStartupArgSpec {
        positive_flags: &["--window-minimized"],
        negative_flags: &["--no-window-minimized"],
        assignment_prefixes: &["--window-minimized="],
        target: BoolStartupArgTarget::WindowMinimized,
    },
];

pub(super) fn parse_bool_startup_arg_legacy_compatible(
    arg: &str,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    for spec in BOOL_STARTUP_ARG_SPECS {
        if spec.positive_flags.contains(&arg) {
            set_bool_startup_arg_legacy_compatible(&mut analysis.parsed, spec.target, true);
            analysis.diagnostics.supported_tokens.push(arg.to_owned());
            return true;
        }
        if spec.negative_flags.contains(&arg) {
            set_bool_startup_arg_legacy_compatible(&mut analysis.parsed, spec.target, false);
            analysis.diagnostics.supported_tokens.push(arg.to_owned());
            return true;
        }
        for prefix in spec.assignment_prefixes {
            if let Some(value) = arg.strip_prefix(prefix) {
                if let Some(parsed) = parse_env_bool_legacy_compatible(value) {
                    set_bool_startup_arg_legacy_compatible(
                        &mut analysis.parsed,
                        spec.target,
                        parsed,
                    );
                    analysis.diagnostics.supported_tokens.push(arg.to_owned());
                } else {
                    analysis.diagnostics.malformed_tokens.push(arg.to_owned());
                }
                return true;
            }
        }
    }
    false
}

fn set_bool_startup_arg_legacy_compatible(
    parsed: &mut LegacyExplicitMpvIpcStartupPlayerArgs,
    target: BoolStartupArgTarget,
    value: bool,
) {
    match target {
        BoolStartupArgTarget::Paused => parsed.paused = Some(value),
        BoolStartupArgTarget::Muted => parsed.muted = Some(value),
        BoolStartupArgTarget::Deinterlace => parsed.deinterlace = Some(value),
        BoolStartupArgTarget::Keepaspect => parsed.keepaspect = Some(value),
        BoolStartupArgTarget::KeepaspectWindow => parsed.keepaspect_window = Some(value),
        BoolStartupArgTarget::Fullscreen => parsed.fullscreen = Some(value),
        BoolStartupArgTarget::Ontop => parsed.ontop = Some(value),
        BoolStartupArgTarget::Border => parsed.border = Some(value),
        BoolStartupArgTarget::ForceWindow => parsed.force_window = Some(value),
        BoolStartupArgTarget::KeepOpen => parsed.keep_open = Some(value),
        BoolStartupArgTarget::KeepOpenPause => parsed.keep_open_pause = Some(value),
        BoolStartupArgTarget::CursorAutohideFsOnly => {
            parsed.cursor_autohide_fs_only = Some(value);
        }
        BoolStartupArgTarget::StopScreensaver => parsed.stop_screensaver = Some(value),
        BoolStartupArgTarget::SubVisibility => parsed.sub_visibility = Some(value),
        BoolStartupArgTarget::OsdBar => parsed.osd_bar = Some(value),
        BoolStartupArgTarget::WindowMaximized => parsed.window_maximized = Some(value),
        BoolStartupArgTarget::WindowMinimized => parsed.window_minimized = Some(value),
    }
}

pub(super) fn parse_start_position_arg_legacy_compatible(
    player_args: &[String],
    index: &mut usize,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    parse_value_option_arg_legacy_compatible(
        player_args,
        index,
        analysis,
        "--start",
        "--start=",
        parse_legacy_explicit_mpv_ipc_start_position_seconds_legacy_compatible,
        |parsed, value| parsed.start_position_seconds = Some(value),
    )
}

pub(super) fn parse_speed_arg_legacy_compatible(
    player_args: &[String],
    index: &mut usize,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    parse_value_option_arg_legacy_compatible(
        player_args,
        index,
        analysis,
        "--speed",
        "--speed=",
        parse_positive_f64_legacy_compatible,
        |parsed, value| parsed.playback_rate = Some(value),
    )
}

pub(super) fn parse_volume_arg_legacy_compatible(
    player_args: &[String],
    index: &mut usize,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    parse_value_option_arg_legacy_compatible(
        player_args,
        index,
        analysis,
        "--volume",
        "--volume=",
        parse_env_non_negative_f64_legacy_compatible,
        |parsed, value| parsed.volume = Some(value),
    )
}

fn parse_value_option_arg_legacy_compatible<P, A>(
    player_args: &[String],
    index: &mut usize,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
    flag: &str,
    assignment_prefix: &str,
    parse_value: P,
    apply_value: A,
) -> bool
where
    P: Fn(&str) -> Option<f64>,
    A: Fn(&mut LegacyExplicitMpvIpcStartupPlayerArgs, f64),
{
    let arg = player_args[*index].as_str();
    if let Some(value) = arg.strip_prefix(assignment_prefix) {
        if let Some(parsed_value) = parse_value(value) {
            apply_value(&mut analysis.parsed, parsed_value);
            analysis.diagnostics.supported_tokens.push(arg.to_owned());
        } else {
            analysis.diagnostics.malformed_tokens.push(arg.to_owned());
        }
        *index += 1;
        return true;
    }
    if arg != flag {
        return false;
    }

    if let Some(next) = player_args.get(*index + 1) {
        if next.starts_with("--") {
            analysis.diagnostics.malformed_tokens.push(arg.to_owned());
            *index += 1;
            return true;
        }
        let combined =
            format_legacy_explicit_mpv_ipc_flag_and_value_token_legacy_compatible(arg, next);
        if let Some(parsed_value) = parse_value(next) {
            apply_value(&mut analysis.parsed, parsed_value);
            analysis.diagnostics.supported_tokens.push(combined);
        } else {
            analysis.diagnostics.malformed_tokens.push(combined);
        }
        *index += 2;
        return true;
    }

    analysis.diagnostics.malformed_tokens.push(arg.to_owned());
    *index += 1;
    true
}

pub(super) fn parse_profile_arg_legacy_compatible(
    player_args: &[String],
    index: &mut usize,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    let arg = player_args[*index].as_str();
    if let Some(value) = arg.strip_prefix("--profile=") {
        if value.trim().is_empty() {
            analysis.diagnostics.malformed_tokens.push(arg.to_owned());
        } else {
            push_legacy_explicit_mpv_ipc_startup_player_command_legacy_compatible(
                &mut analysis.runtime_commands,
                LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile {
                    profile: value.to_owned(),
                },
            );
            analysis.diagnostics.supported_tokens.push(arg.to_owned());
        }
        *index += 1;
        return true;
    }
    if arg != "--profile" {
        return false;
    }

    if let Some(next) = player_args.get(*index + 1) {
        if next.starts_with("--") {
            analysis.diagnostics.malformed_tokens.push(arg.to_owned());
            *index += 1;
            return true;
        }
        push_legacy_explicit_mpv_ipc_startup_player_command_legacy_compatible(
            &mut analysis.runtime_commands,
            LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile {
                profile: next.to_owned(),
            },
        );
        analysis
            .diagnostics
            .supported_tokens
            .push(format_legacy_explicit_mpv_ipc_flag_and_value_token_legacy_compatible(arg, next));
        *index += 2;
        return true;
    }

    analysis.diagnostics.malformed_tokens.push(arg.to_owned());
    *index += 1;
    true
}

pub(super) fn parse_generic_option_assignment_arg_legacy_compatible(
    arg: &str,
    analysis: &mut LegacyExplicitMpvIpcStartupPlayerArgAnalysis,
) -> bool {
    if let Some((name, value)) =
        parse_legacy_explicit_mpv_ipc_generic_option_assignment_legacy_compatible(arg)
    {
        push_legacy_explicit_mpv_ipc_startup_player_command_legacy_compatible(
            &mut analysis.runtime_commands,
            LegacyExplicitMpvIpcStartupPlayerCommand::SetOptionString { name, value },
        );
        analysis.diagnostics.supported_tokens.push(arg.to_owned());
        return true;
    }
    false
}

fn parse_positive_f64_legacy_compatible(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    (parsed.is_finite() && parsed > 0.0).then_some(parsed)
}

fn parse_legacy_explicit_mpv_ipc_start_position_seconds_legacy_compatible(
    value: &str,
) -> Option<f64> {
    parse_env_non_negative_f64_legacy_compatible(value)
        .or_else(|| parse_seek_time_seconds_legacy_like(value))
}

fn format_legacy_explicit_mpv_ipc_flag_and_value_token_legacy_compatible(
    flag: &str,
    value: &str,
) -> String {
    format!("{flag} {value}")
}

fn parse_legacy_explicit_mpv_ipc_generic_option_assignment_legacy_compatible(
    arg: &str,
) -> Option<(String, String)> {
    let option = arg
        .strip_prefix("--")
        .or_else(|| arg.strip_prefix('-'))
        .filter(|option| !option.is_empty())?;
    let (name, value) = option.split_once('=')?;
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return None;
    }
    Some((trimmed_name.to_owned(), value.to_owned()))
}

fn legacy_explicit_mpv_ipc_startup_player_command_key_legacy_compatible(
    command: &LegacyExplicitMpvIpcStartupPlayerCommand,
) -> (&str, Option<&str>) {
    match command {
        LegacyExplicitMpvIpcStartupPlayerCommand::SetOptionString { name, .. } => {
            ("set-option", Some(name.as_str()))
        }
        LegacyExplicitMpvIpcStartupPlayerCommand::ApplyProfile { .. } => ("apply-profile", None),
    }
}

fn push_legacy_explicit_mpv_ipc_startup_player_command_legacy_compatible(
    commands: &mut Vec<LegacyExplicitMpvIpcStartupPlayerCommand>,
    command: LegacyExplicitMpvIpcStartupPlayerCommand,
) {
    let command_key =
        legacy_explicit_mpv_ipc_startup_player_command_key_legacy_compatible(&command);
    if let Some(existing_index) = commands.iter().position(|existing| {
        legacy_explicit_mpv_ipc_startup_player_command_key_legacy_compatible(existing)
            == command_key
    }) {
        commands.remove(existing_index);
    }
    commands.push(command);
}
