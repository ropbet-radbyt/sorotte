use std::env;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use anyhow::Context;

const DEFAULT_SERVER_PORT: u16 = 8999;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ServerCliOverrides {
    port: Option<u16>,
    password: Option<String>,
    salt: Option<String>,
    motd_file: Option<PathBuf>,
    rooms_db_file: Option<PathBuf>,
    permanent_rooms_file: Option<PathBuf>,
    stats_db_file: Option<PathBuf>,
    tls_cert_path: Option<PathBuf>,
    disable_ready: bool,
    disable_chat: bool,
    max_chat_message_length: Option<usize>,
    max_username_length: Option<usize>,
    isolate_rooms: bool,
    ipv4_only: bool,
    ipv6_only: bool,
    interface_ipv4: Option<String>,
    interface_ipv6: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    Run(Box<ServerCliOverrides>),
    Help,
    Version,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerRunConfig {
    pub(crate) bind_host: String,
    pub(crate) port: u16,
    pub(crate) server_password_token: Option<String>,
    pub(crate) room_password_salt: Option<String>,
    pub(crate) motd_template: Option<String>,
    pub(crate) rooms_db_file: Option<PathBuf>,
    pub(crate) permanent_rooms_file: Option<PathBuf>,
    pub(crate) stats_db_file: Option<PathBuf>,
    pub(crate) tls_cert_path: Option<PathBuf>,
    pub(crate) persistent_rooms_enabled: bool,
    pub(crate) chat_enabled: bool,
    pub(crate) readiness_enabled: bool,
    pub(crate) max_chat_message_length: Option<usize>,
    pub(crate) max_username_length: Option<usize>,
    pub(crate) isolate_rooms: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliParseError(String);

impl CliParseError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliParseError {}

fn env_flag_enabled(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

fn env_trimmed(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn env_u16(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
}

fn help_text() -> &'static str {
    concat!(
        "syncplay-server (Rust alpha)\n\n",
        "Usage:\n",
        "  syncplay-server [options]\n\n",
        "Supported options:\n",
        "  -h, --help                       Show this help text\n",
        "  -V, --version                    Show version\n",
        "  --port <port>                    TCP port to listen on (default: 8999)\n",
        "  --password <password>            Server password token (exact compare in current alpha)\n",
        "  --salt <salt>                    Salt for controlled-room password hashes\n",
        "  --disable-ready                  Disable readiness feature\n",
        "  --disable-chat                   Disable chat feature\n",
        "  --isolate-rooms                  Isolate rooms (room-scoped visibility/listing)\n",
        "  --motd-file <file>               Read MOTD template from file\n",
        "  --rooms-db-file <file>           Enable persistent rooms using SQLite file\n",
        "  --permanent-rooms-file <file>    Load permanent room names (one per line)\n",
        "  --max-chat-message-length <n>    Advertised/applied chat message length cap\n",
        "  --max-username-length <n>        Advertised/applied username length cap\n",
        "  --stats-db-file <file>           Enable stats snapshots using SQLite file\n",
        "  --tls <dir>                      TLS certificate directory (cert.pem/chain.pem/privkey.pem)\n",
        "  --ipv4-only                      Bind IPv4 listen socket (default behavior)\n",
        "  --ipv6-only                      Bind IPv6 listen socket\n",
        "  --interface-ipv4 <ip>            Bind to specific IPv4 address\n",
        "  --interface-ipv6 <ip>            Bind to specific IPv6 address\n\n",
        "Environment overrides (legacy bootstrap compatibility):\n",
        "  SYNCPLAY_SERVER_PORT\n",
        "  SYNCPLAY_SERVER_PASSWORD (or SYNCPLAY_PASSWORD; exact compare token in current alpha)\n",
        "  SYNCPLAY_SERVER_SALT (or SYNCPLAY_SALT)\n",
        "  SYNCPLAY_SERVER_MOTD_TEMPLATE\n",
        "  SYNCPLAY_SERVER_ROOMS_DB_FILE\n",
        "  SYNCPLAY_SERVER_PERMANENT_ROOMS_FILE\n",
        "  SYNCPLAY_SERVER_STATS_DB_FILE\n",
        "  SYNCPLAY_SERVER_TLS_CERT_PATH\n",
        "  SYNCPLAY_SERVER_PERSISTENT_ROOMS\n",
    )
}

fn take_option_value(
    inline_value: Option<String>,
    args: &[String],
    index: &mut usize,
    option_name: &str,
) -> Result<String, CliParseError> {
    if let Some(value) = inline_value {
        return Ok(value);
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliParseError::new(format!(
            "{option_name} requires a value (see --help)"
        )));
    };
    if value.starts_with('-') {
        return Err(CliParseError::new(format!(
            "{option_name} requires a value (got option '{value}')"
        )));
    }
    *index += 1;
    Ok(value.clone())
}

fn split_long_option(arg: &str) -> Option<(&str, Option<String>)> {
    if !arg.starts_with("--") {
        return None;
    }
    match arg.split_once('=') {
        Some((name, value)) => Some((name, Some(value.to_owned()))),
        None => Some((arg, None)),
    }
}

pub(crate) fn parse_server_cli_args<I, S>(args: I) -> Result<CliAction, CliParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut overrides = ServerCliOverrides::default();
    let mut unknown_options: Vec<String> = Vec::new();
    let mut positional_args: Vec<String> = Vec::new();
    let mut index = 0usize;

    while index < args.len() {
        let arg = &args[index];

        if arg == "-h" || arg == "--help" {
            return Ok(CliAction::Help);
        }
        if arg == "-V" || arg == "--version" {
            return Ok(CliAction::Version);
        }
        if arg == "--" {
            positional_args.extend(args[(index + 1)..].iter().cloned());
            break;
        }

        let Some((name, inline_value)) = split_long_option(arg) else {
            if arg.starts_with('-') {
                unknown_options.push(arg.clone());
            } else {
                positional_args.push(arg.clone());
            }
            index += 1;
            continue;
        };

        match name {
            "--port" => {
                let value = take_option_value(inline_value, &args, &mut index, "--port")?;
                let parsed = value.parse::<u16>().map_err(|_| {
                    CliParseError::new(format!(
                        "--port expects an integer in 0..=65535, got '{value}'"
                    ))
                })?;
                overrides.port = Some(parsed);
            }
            "--password" => {
                let value = take_option_value(inline_value, &args, &mut index, "--password")?;
                overrides.password = Some(value);
            }
            "--salt" => {
                let value = take_option_value(inline_value, &args, &mut index, "--salt")?;
                overrides.salt = Some(value);
            }
            "--disable-ready" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--disable-ready does not take a value"));
                }
                overrides.disable_ready = true;
            }
            "--disable-chat" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--disable-chat does not take a value"));
                }
                overrides.disable_chat = true;
            }
            "--isolate-rooms" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--isolate-rooms does not take a value"));
                }
                overrides.isolate_rooms = true;
            }
            "--motd-file" => {
                let value = take_option_value(inline_value, &args, &mut index, "--motd-file")?;
                overrides.motd_file = Some(PathBuf::from(value));
            }
            "--rooms-db-file" => {
                let value = take_option_value(inline_value, &args, &mut index, "--rooms-db-file")?;
                overrides.rooms_db_file = Some(PathBuf::from(value));
            }
            "--permanent-rooms-file" => {
                let value =
                    take_option_value(inline_value, &args, &mut index, "--permanent-rooms-file")?;
                overrides.permanent_rooms_file = Some(PathBuf::from(value));
            }
            "--stats-db-file" => {
                let value = take_option_value(inline_value, &args, &mut index, "--stats-db-file")?;
                overrides.stats_db_file = Some(PathBuf::from(value));
            }
            "--max-chat-message-length" => {
                let value = take_option_value(
                    inline_value,
                    &args,
                    &mut index,
                    "--max-chat-message-length",
                )?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    CliParseError::new(format!(
                        "--max-chat-message-length expects a non-negative integer, got '{value}'"
                    ))
                })?;
                overrides.max_chat_message_length = Some(parsed);
            }
            "--max-username-length" => {
                let value =
                    take_option_value(inline_value, &args, &mut index, "--max-username-length")?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    CliParseError::new(format!(
                        "--max-username-length expects a non-negative integer, got '{value}'"
                    ))
                })?;
                overrides.max_username_length = Some(parsed);
            }
            "--tls" => {
                let value = take_option_value(inline_value, &args, &mut index, "--tls")?;
                overrides.tls_cert_path = Some(PathBuf::from(value));
            }
            "--ipv4-only" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--ipv4-only does not take a value"));
                }
                overrides.ipv4_only = true;
            }
            "--ipv6-only" => {
                if inline_value.is_some() {
                    return Err(CliParseError::new("--ipv6-only does not take a value"));
                }
                overrides.ipv6_only = true;
            }
            "--interface-ipv4" => {
                let value = take_option_value(inline_value, &args, &mut index, "--interface-ipv4")?;
                overrides.interface_ipv4 = Some(value);
            }
            "--interface-ipv6" => {
                let value = take_option_value(inline_value, &args, &mut index, "--interface-ipv6")?;
                overrides.interface_ipv6 = Some(value);
            }
            _ => {
                unknown_options.push(arg.clone());
            }
        }

        index += 1;
    }

    if !unknown_options.is_empty() {
        return Err(CliParseError::new(format!(
            "unknown option(s): {} (see --help)",
            unknown_options.join(", ")
        )));
    }

    if !positional_args.is_empty() {
        return Err(CliParseError::new(format!(
            "positional arguments are not supported yet: {}",
            positional_args.join(" ")
        )));
    }

    Ok(CliAction::Run(Box::new(overrides)))
}

fn resolve_bind_host(overrides: &ServerCliOverrides) -> Result<String, CliParseError> {
    if overrides.ipv4_only && overrides.ipv6_only {
        return Err(CliParseError::new(
            "--ipv4-only and --ipv6-only are mutually exclusive",
        ));
    }
    if overrides.interface_ipv4.is_some() && overrides.interface_ipv6.is_some() {
        return Err(CliParseError::new(
            "binding both --interface-ipv4 and --interface-ipv6 is not implemented yet",
        ));
    }
    if overrides.ipv4_only && overrides.interface_ipv6.is_some() {
        return Err(CliParseError::new(
            "--ipv4-only cannot be used with --interface-ipv6",
        ));
    }
    if overrides.ipv6_only && overrides.interface_ipv4.is_some() {
        return Err(CliParseError::new(
            "--ipv6-only cannot be used with --interface-ipv4",
        ));
    }

    if let Some(ipv4) = overrides.interface_ipv4.as_deref() {
        let parsed = ipv4.parse::<Ipv4Addr>().map_err(|_| {
            CliParseError::new(format!(
                "--interface-ipv4 expects an IPv4 address, got '{ipv4}'"
            ))
        })?;
        return Ok(parsed.to_string());
    }
    if let Some(ipv6) = overrides.interface_ipv6.as_deref() {
        let parsed = ipv6.parse::<Ipv6Addr>().map_err(|_| {
            CliParseError::new(format!(
                "--interface-ipv6 expects an IPv6 address, got '{ipv6}'"
            ))
        })?;
        return Ok(parsed.to_string());
    }

    if overrides.ipv6_only {
        Ok(Ipv6Addr::UNSPECIFIED.to_string())
    } else {
        Ok(Ipv4Addr::UNSPECIFIED.to_string())
    }
}

pub(crate) fn resolve_run_config(
    overrides: ServerCliOverrides,
) -> Result<ServerRunConfig, anyhow::Error> {
    let bind_host = resolve_bind_host(&overrides)?;
    let port = overrides
        .port
        .or_else(|| env_u16("SYNCPLAY_SERVER_PORT"))
        .unwrap_or(DEFAULT_SERVER_PORT);
    let room_password_salt = overrides
        .salt
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_SALT"))
        .or_else(|| env_trimmed("SYNCPLAY_SALT"));
    let server_password_token = overrides
        .password
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_PASSWORD"))
        .or_else(|| env_trimmed("SYNCPLAY_PASSWORD"));

    let motd_template = if let Some(motd_file) = overrides.motd_file {
        Some(
            fs::read_to_string(&motd_file)
                .with_context(|| format!("failed to read MOTD file '{}'", motd_file.display()))?,
        )
    } else {
        env_trimmed("SYNCPLAY_SERVER_MOTD_TEMPLATE")
    };

    let rooms_db_file = overrides
        .rooms_db_file
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_ROOMS_DB_FILE").map(PathBuf::from));
    let permanent_rooms_file = overrides
        .permanent_rooms_file
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_PERMANENT_ROOMS_FILE").map(PathBuf::from));
    let stats_db_file = overrides
        .stats_db_file
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_STATS_DB_FILE").map(PathBuf::from));
    let tls_cert_path = overrides
        .tls_cert_path
        .or_else(|| env_trimmed("SYNCPLAY_SERVER_TLS_CERT_PATH").map(PathBuf::from));

    let persistent_rooms_enabled = env_flag_enabled("SYNCPLAY_SERVER_PERSISTENT_ROOMS")
        || rooms_db_file.is_some()
        || permanent_rooms_file.is_some();

    Ok(ServerRunConfig {
        bind_host,
        port,
        server_password_token,
        room_password_salt,
        motd_template,
        rooms_db_file,
        permanent_rooms_file,
        stats_db_file,
        tls_cert_path,
        persistent_rooms_enabled,
        chat_enabled: !overrides.disable_chat,
        readiness_enabled: !overrides.disable_ready,
        max_chat_message_length: overrides.max_chat_message_length,
        max_username_length: overrides.max_username_length,
        isolate_rooms: overrides.isolate_rooms,
    })
}

pub(crate) fn print_help() {
    println!("{}", help_text());
}

pub(crate) fn print_version() {
    println!("syncplay-server {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_flag_short_circuits_parsing() {
        assert_eq!(parse_server_cli_args(["--help"]).unwrap(), CliAction::Help);
    }

    #[test]
    fn parses_supported_server_flags() {
        let action = parse_server_cli_args([
            "--port",
            "9000",
            "--password",
            "secret",
            "--salt=pepper",
            "--disable-chat",
            "--disable-ready",
            "--rooms-db-file",
            "rooms.sqlite3",
            "--permanent-rooms-file=permanent.txt",
            "--max-chat-message-length",
            "42",
            "--max-username-length=12",
            "--stats-db-file",
            "stats.sqlite3",
            "--tls",
            "certs",
            "--ipv6-only",
            "--interface-ipv6",
            "::1",
        ])
        .unwrap();

        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };
        let overrides = *overrides;
        assert_eq!(overrides.port, Some(9000));
        assert_eq!(overrides.password.as_deref(), Some("secret"));
        assert_eq!(overrides.salt.as_deref(), Some("pepper"));
        assert!(overrides.disable_chat);
        assert!(overrides.disable_ready);
        assert!(!overrides.isolate_rooms);
        assert_eq!(overrides.max_chat_message_length, Some(42));
        assert_eq!(overrides.max_username_length, Some(12));
        assert_eq!(
            overrides.rooms_db_file,
            Some(PathBuf::from("rooms.sqlite3"))
        );
        assert_eq!(
            overrides.permanent_rooms_file,
            Some(PathBuf::from("permanent.txt"))
        );
        assert_eq!(
            overrides.stats_db_file,
            Some(PathBuf::from("stats.sqlite3"))
        );
        assert_eq!(overrides.tls_cert_path, Some(PathBuf::from("certs")));
        assert!(overrides.ipv6_only);
        assert_eq!(overrides.interface_ipv6.as_deref(), Some("::1"));
    }

    #[test]
    fn parses_supported_server_flags_in_isolate_mode_without_persistence() {
        let action = parse_server_cli_args([
            "--port",
            "9000",
            "--password",
            "secret",
            "--salt=pepper",
            "--disable-chat",
            "--disable-ready",
            "--isolate-rooms",
            "--max-chat-message-length",
            "42",
            "--max-username-length=12",
            "--stats-db-file",
            "stats.sqlite3",
            "--tls",
            "certs",
            "--ipv6-only",
            "--interface-ipv6",
            "::1",
        ])
        .unwrap();

        let CliAction::Run(overrides) = action else {
            panic!("expected run action");
        };
        let overrides = *overrides;
        assert_eq!(overrides.port, Some(9000));
        assert_eq!(overrides.password.as_deref(), Some("secret"));
        assert_eq!(overrides.salt.as_deref(), Some("pepper"));
        assert!(overrides.disable_chat);
        assert!(overrides.disable_ready);
        assert!(overrides.isolate_rooms);
        assert_eq!(overrides.max_chat_message_length, Some(42));
        assert_eq!(overrides.max_username_length, Some(12));
        assert_eq!(overrides.rooms_db_file, None);
        assert_eq!(overrides.permanent_rooms_file, None);
        assert_eq!(
            overrides.stats_db_file,
            Some(PathBuf::from("stats.sqlite3"))
        );
        assert_eq!(overrides.tls_cert_path, Some(PathBuf::from("certs")));
        assert!(overrides.ipv6_only);
        assert_eq!(overrides.interface_ipv6.as_deref(), Some("::1"));
    }

    #[test]
    fn unknown_flags_are_reported() {
        let error = parse_server_cli_args(["--definitely-not-a-real-flag"]).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("--definitely-not-a-real-flag"));
    }

    #[test]
    fn isolate_mode_accepts_persistent_room_configuration() {
        let overrides = ServerCliOverrides {
            isolate_rooms: true,
            rooms_db_file: Some(PathBuf::from("rooms.sqlite3")),
            ..ServerCliOverrides::default()
        };
        let config = resolve_run_config(overrides).unwrap();
        assert!(config.isolate_rooms);
        assert!(config.persistent_rooms_enabled);
        assert_eq!(config.rooms_db_file, Some(PathBuf::from("rooms.sqlite3")));
    }

    #[test]
    fn bind_host_conflict_is_rejected() {
        let overrides = ServerCliOverrides {
            ipv4_only: true,
            ipv6_only: true,
            ..ServerCliOverrides::default()
        };
        let error = resolve_bind_host(&overrides).unwrap_err();
        assert!(error.to_string().contains("mutually exclusive"));
    }
}
