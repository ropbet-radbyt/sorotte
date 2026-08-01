use super::*;

#[cfg(test)]
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

fn attached_long_option_value<'a>(arg: &'a str, long: &str) -> Option<&'a str> {
    arg.strip_prefix(long)
        .and_then(|suffix| suffix.strip_prefix('='))
}

fn parse_explicit_port(port: &str) -> Result<u16, HostArgumentError> {
    let port = port.trim();
    if port.is_empty() {
        return Err(HostArgumentError::EmptyPort);
    }
    match port.parse::<i128>() {
        Ok(value @ 1..=65_535) => Ok(value as u16),
        Ok(_) => Err(HostArgumentError::PortOutOfRange),
        Err(_) => {
            let unsigned = port
                .strip_prefix('+')
                .or_else(|| port.strip_prefix('-'))
                .unwrap_or(port);
            if !unsigned.is_empty() && unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
                Err(HostArgumentError::PortOutOfRange)
            } else {
                Err(HostArgumentError::NonNumericPort)
            }
        }
    }
}

fn parse_cli_host_argument(value: &str) -> Result<(String, Option<u16>), HostArgumentError> {
    if value.starts_with('[') {
        let closing_bracket = value
            .find(']')
            .ok_or(HostArgumentError::MalformedBracketedIpv6)?;
        if closing_bracket == 1 {
            return Err(HostArgumentError::EmptyHost);
        }
        let host = &value[..=closing_bracket];
        let suffix = &value[closing_bracket + 1..];
        if suffix.is_empty() {
            return Ok((host.to_owned(), None));
        }
        let port = suffix
            .strip_prefix(':')
            .ok_or(HostArgumentError::MalformedBracketedIpv6)?;
        return Ok((host.to_owned(), Some(parse_explicit_port(port)?)));
    }
    if value.contains('[') || value.contains(']') {
        return Err(HostArgumentError::MalformedBracketedIpv6);
    }

    match value.bytes().filter(|byte| *byte == b':').count() {
        0 => {
            if value.is_empty() {
                Err(HostArgumentError::EmptyHost)
            } else {
                Ok((value.to_owned(), None))
            }
        }
        1 => {
            let (host, port) = value
                .split_once(':')
                .expect("one-colon host must split once");
            if host.is_empty() {
                return Err(HostArgumentError::EmptyHost);
            }
            Ok((host.to_owned(), Some(parse_explicit_port(port)?)))
        }
        _ => Ok((format!("[{value}]"), None)),
    }
}

fn replace_host_override(overrides: &mut LegacyClientArgOverrides, option: &str, value: &str) {
    overrides.host = None;
    overrides.port = None;
    overrides
        .unknown_options
        .retain(|issue| !issue.is_host_argument());
    if value.is_empty() {
        return;
    }
    match parse_cli_host_argument(value) {
        Ok((host, port)) => {
            overrides.host = Some(host);
            overrides.port = port;
        }
        Err(error) => {
            overrides
                .unknown_options
                .push(LegacyClientArgumentIssue::invalid_host(option, error));
        }
    }
}

fn replace_non_empty_override(target: &mut Option<String>, value: &str) {
    *target = (!value.is_empty()).then(|| value.to_owned());
}

fn replace_password_override(overrides: &mut LegacyClientArgOverrides, value: &str) {
    overrides.controlled_room_password_override =
        (!value.is_empty()).then(|| SecretValue::from(value.to_owned()));
}

fn parse_short_option_token<I>(
    arg: &str,
    args: &mut std::iter::Peekable<I>,
    overrides: &mut LegacyClientArgOverrides,
) -> bool
where
    I: Iterator<Item = String>,
{
    if !arg.starts_with('-') || arg.starts_with("--") || arg.len() == 1 {
        return false;
    }

    let body = &arg[1..];
    let mut cursor = 0usize;
    let mut staged = overrides.clone();
    while cursor < body.len() {
        let option = body[cursor..]
            .chars()
            .next()
            .expect("short-option cursor must remain on a character boundary");
        cursor += option.len_utf8();
        let remainder = &body[cursor..];
        match option {
            'h' => staged.show_help = true,
            'v' => staged.show_version = true,
            'd' => staged.debug_requested = true,
            'g' => staged.force_gui_prompt_requested = true,
            'a' | 'n' | 'r' | 'p' => {
                staged.connect_requested = true;
                let option_name = format!("-{option}");
                let attached_value = (!remainder.is_empty())
                    .then(|| remainder.strip_prefix('=').unwrap_or(remainder).to_owned());
                let value =
                    attached_value.or_else(|| take_next_non_flag_arg_legacy_compatible(args));
                match option {
                    'a' => match value {
                        Some(value) => replace_host_override(&mut staged, &option_name, &value),
                        None => staged
                            .unknown_options
                            .push(LegacyClientArgumentIssue::missing_value(&option_name)),
                    },
                    'n' => match value {
                        Some(value) => replace_non_empty_override(&mut staged.username, &value),
                        None => staged
                            .unknown_options
                            .push(LegacyClientArgumentIssue::missing_value(&option_name)),
                    },
                    'r' => replace_non_empty_override(
                        &mut staged.room,
                        value.as_deref().unwrap_or_default(),
                    ),
                    'p' => {
                        replace_password_override(&mut staged, value.as_deref().unwrap_or_default())
                    }
                    _ => unreachable!("matched short value option"),
                }
                *overrides = staged;
                return true;
            }
            unknown => {
                overrides
                    .unknown_options
                    .push(LegacyClientArgumentIssue::unknown_short_option(
                        unknown,
                        !remainder.is_empty(),
                    ));
                return true;
            }
        }
    }
    *overrides = staged;
    true
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
        if let Some(value) = attached_long_option_value(&arg, "--host") {
            overrides.connect_requested = true;
            replace_host_override(&mut overrides, "--host", value);
            continue;
        }
        if let Some(value) = attached_long_option_value(&arg, "--name") {
            overrides.connect_requested = true;
            replace_non_empty_override(&mut overrides.username, value);
            continue;
        }
        if let Some(value) = attached_long_option_value(&arg, "--room") {
            overrides.connect_requested = true;
            replace_non_empty_override(&mut overrides.room, value);
            continue;
        }
        if let Some(value) = attached_long_option_value(&arg, "--password") {
            overrides.connect_requested = true;
            replace_password_override(&mut overrides, value);
            continue;
        }
        match arg.as_str() {
            "--help" => {
                overrides.show_help = true;
            }
            "--version" => {
                overrides.show_version = true;
            }
            "--debug" => {
                overrides.debug_requested = true;
            }
            "--force-gui-prompt" => {
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
                if take_next_non_flag_arg_legacy_compatible(&mut iter).is_none() {
                    overrides
                        .unknown_options
                        .push(LegacyClientArgumentIssue::missing_value("-psn"));
                }
            }
            value if value.starts_with("-psn=") => {
                // macOS process-serial-number compatibility black hole.
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
            "--host" => {
                overrides.connect_requested = true;
                if let Some(value) = take_next_non_flag_arg_legacy_compatible(&mut iter) {
                    replace_host_override(&mut overrides, "--host", &value);
                } else {
                    overrides
                        .unknown_options
                        .push(LegacyClientArgumentIssue::missing_value("--host"));
                }
            }
            "--name" => {
                overrides.connect_requested = true;
                if let Some(value) = take_next_non_flag_arg_legacy_compatible(&mut iter) {
                    replace_non_empty_override(&mut overrides.username, &value);
                } else {
                    overrides
                        .unknown_options
                        .push(LegacyClientArgumentIssue::missing_value("--name"));
                }
            }
            "--room" => {
                overrides.connect_requested = true;
                overrides.room = take_next_non_flag_arg_legacy_compatible(&mut iter)
                    .filter(|value| !value.is_empty());
            }
            "--password" => {
                overrides.connect_requested = true;
                overrides.controlled_room_password_override =
                    take_next_non_flag_arg_legacy_compatible(&mut iter)
                        .filter(|value| !value.is_empty())
                        .map(SecretValue::from);
            }
            _ => {
                if parse_short_option_token(&arg, &mut iter, &mut overrides) {
                    continue;
                } else if arg.starts_with('-') {
                    overrides
                        .unknown_options
                        .push(LegacyClientArgumentIssue::unknown_option(&arg));
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
