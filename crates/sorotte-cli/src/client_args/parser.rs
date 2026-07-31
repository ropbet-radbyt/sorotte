use super::*;

pub(crate) fn parse_host_and_optional_port_from_host_arg_legacy_compatible(
    host_value: &str,
) -> (String, Option<u16>) {
    shared_parse_host_and_optional_port_from_host_arg_legacy_compatible(host_value)
}

fn take_next_non_flag_arg_legacy_compatible<I>(args: &mut std::iter::Peekable<I>) -> Option<String>
where
    I: Iterator<Item = String>,
{
    if args.peek().is_some_and(|value| !value.starts_with('-')) {
        return args.next();
    }
    None
}

fn attached_option_value<'a>(arg: &'a str, long: &str, short: &str) -> Option<&'a str> {
    arg.strip_prefix(long)
        .and_then(|suffix| suffix.strip_prefix('='))
        .or_else(|| {
            arg.strip_prefix(short)
                .and_then(|suffix| suffix.strip_prefix('='))
        })
}

fn replace_host_override(overrides: &mut LegacyClientArgOverrides, value: &str) {
    overrides.host = None;
    overrides.port = None;
    if value.is_empty() {
        return;
    }
    let (host, port) = parse_host_and_optional_port_from_host_arg_legacy_compatible(value);
    if !host.is_empty() {
        overrides.host = Some(host);
        overrides.port = port;
    }
}

fn replace_non_empty_override(target: &mut Option<String>, value: &str) {
    *target = (!value.is_empty()).then(|| value.to_owned());
}

fn replace_password_override(overrides: &mut LegacyClientArgOverrides, value: &str) {
    overrides.controlled_room_password_override =
        (!value.is_empty()).then(|| SecretValue::from(value.to_owned()));
}

pub(crate) fn parse_legacy_client_arg_overrides<I, S>(args: I) -> LegacyClientArgOverrides
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut overrides = LegacyClientArgOverrides::default();
    let mut iter = args
        .into_iter()
        .map(|value| value.as_ref().to_owned())
        .peekable();

    while let Some(arg) = iter.next() {
        if arg == "--" {
            let mut trailing_args = iter.collect::<Vec<_>>();
            if !trailing_args.is_empty() {
                overrides.connect_requested = true;
                if overrides.file.is_none() {
                    let first = trailing_args.remove(0);
                    if first.starts_with("--") {
                        // Python ConfigurationGetter (argparse) fills positional `file` first after `--`,
                        // then rewrites `file="--foo"` into the player-args list.
                        overrides.player_args.push(first);
                    } else {
                        overrides.file = Some(first);
                    }
                }
                overrides.player_args.extend(trailing_args);
            }
            break;
        }
        if let Some(value) = attached_option_value(&arg, "--host", "-a") {
            overrides.connect_requested = true;
            replace_host_override(&mut overrides, value);
            continue;
        }
        if let Some(value) = attached_option_value(&arg, "--name", "-n") {
            overrides.connect_requested = true;
            replace_non_empty_override(&mut overrides.username, value);
            continue;
        }
        if let Some(value) = attached_option_value(&arg, "--room", "-r") {
            overrides.connect_requested = true;
            replace_non_empty_override(&mut overrides.room, value);
            continue;
        }
        if let Some(value) = attached_option_value(&arg, "--password", "-p") {
            overrides.connect_requested = true;
            replace_password_override(&mut overrides, value);
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                overrides.show_help = true;
            }
            "-v" | "--version" => {
                overrides.show_version = true;
            }
            "-d" | "--debug" => {
                overrides.debug_requested = true;
            }
            "-g" | "--force-gui-prompt" => {
                overrides.force_gui_prompt_requested = true;
            }
            "--clear-gui-data" => {
                overrides.clear_gui_data_requested = true;
            }
            "--config-path" => {
                overrides.config_path = take_next_non_flag_arg_legacy_compatible(&mut iter);
            }
            "--config-root" => {
                overrides.config_root = take_next_non_flag_arg_legacy_compatible(&mut iter);
            }
            "--no-store" => {
                overrides.no_store = true;
            }
            "-psn" => {
                let _ = iter.next();
            }
            "--language" => {
                overrides.language = take_next_non_flag_arg_legacy_compatible(&mut iter);
            }
            "--player-path" => {
                overrides.connect_requested = true;
                overrides.player_path = take_next_non_flag_arg_legacy_compatible(&mut iter);
            }
            "--load-playlist-from-file" => {
                overrides.load_playlist_from_file =
                    take_next_non_flag_arg_legacy_compatible(&mut iter);
            }
            "--no-gui" => {
                overrides.connect_requested = true;
                overrides.no_gui_requested = true;
            }
            "-a" | "--host" => {
                overrides.connect_requested = true;
                if let Some(value) = take_next_non_flag_arg_legacy_compatible(&mut iter) {
                    replace_host_override(&mut overrides, &value);
                } else {
                    overrides.unknown_options.push(arg);
                }
            }
            "-n" | "--name" => {
                overrides.connect_requested = true;
                if let Some(value) = take_next_non_flag_arg_legacy_compatible(&mut iter) {
                    replace_non_empty_override(&mut overrides.username, &value);
                } else {
                    overrides.unknown_options.push(arg);
                }
            }
            "-r" | "--room" => {
                overrides.connect_requested = true;
                overrides.room = take_next_non_flag_arg_legacy_compatible(&mut iter)
                    .filter(|value| !value.is_empty());
            }
            "-p" | "--password" => {
                overrides.connect_requested = true;
                overrides.controlled_room_password_override =
                    take_next_non_flag_arg_legacy_compatible(&mut iter)
                        .filter(|value| !value.is_empty())
                        .map(SecretValue::from);
            }
            _ => {
                if arg.starts_with('-') {
                    overrides.unknown_options.push(arg);
                } else if overrides.file.is_none() {
                    overrides.connect_requested = true;
                    overrides.file = Some(arg);
                } else {
                    overrides.connect_requested = true;
                    overrides.player_args.push(arg);
                }
            }
        }
    }

    overrides
}
